//! Syncing without the desktop client: `sync --once` (one full pass, then exit) and `daemon` (the
//! sync engine without a window or tray, until stopped). Both use the saved settings and sign-in,
//! and claim the sync folder first, so only one engine ever syncs a folder.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craftcloud_core::sync::{ActivityEvent, ClientSettings, SyncPlan, SyncProgress};
use craftcloud_native::control::{self, Activity, Attached, FolderLock, Status};
use craftcloud_native::sync::{Command, Handle};
use serde_json::json;

use super::output::{CliError, Fail, Result, print_json};
use super::remote::Ctx;

/// A running engine and the folder it holds.
struct Engine {
    handle: Handle,
    _lock: FolderLock,
    settings: ClientSettings,
}

fn start(ctx: &Ctx) -> Result<Engine> {
    let settings = ctx.settings();
    if settings.local_folder.is_empty() {
        return Err(CliError::usage("No sync folder yet: set one up in the desktop client first (or in its settings file)."));
    }
    let lock = control::claim_folder(Path::new(&settings.local_folder)).map_err(CliError::conflict)?;
    let remote = ctx.remote()?;
    let mut plan = SyncPlan::from(&settings);
    plan.server = remote.server().to_string();
    let device = Some(settings.device_name.trim().to_owned()).filter(|d| !d.is_empty()).unwrap_or_else(craftcloud_native::device_name);
    let handle = craftcloud_native::sync::start_with(plan, remote.session, device, Arc::new(|| {}));
    Ok(Engine { handle, _lock: lock, settings })
}

fn progress(handle: &Handle) -> SyncProgress {
    handle.progress.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// `sync --once` (and `sync` when no desktop client runs): one full pass.
pub fn once(ctx: &Ctx, deletes: super::account::Deletes) -> Result<()> {
    let engine = start(ctx)?;
    match deletes {
        super::account::Deletes::Hold => {}
        super::account::Deletes::Confirm => engine.handle.send(Command::ConfirmDeletes),
        super::account::Deletes::Discard => engine.handle.send(Command::DiscardDeletes),
    }
    engine.handle.send(Command::SyncFor(1));
    let deadline = Instant::now() + Duration::from_secs(6 * 60 * 60);
    let p = loop {
        let p = progress(&engine.handle);
        if p.served >= 1 || Instant::now() > deadline {
            break p;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    drop(engine.handle);
    let status = Status::from_progress(&p, &engine.settings.server, &engine.settings.local_folder);
    let activity: Vec<Activity> = p.activity.iter().map(Activity::from).collect();
    if ctx.json {
        print_json(&json!({
            "state": status.state,
            "error": status.error,
            "pending_deletes": status.pending_deletes,
            "files_done": p.files_done,
            "files_total": p.files_total,
            "conflicts": status.conflicts,
            "not_synced": status.not_synced,
            "activity": activity,
        }))?;
    } else {
        for a in &activity {
            println!("{}", a.line());
        }
        if status.state == "up_to_date" {
            println!("{}", super::account::state_words(&status));
        }
    }
    super::account::failed(&status)
}

/// The events that are new since the last look (`seen` of them, the last one `last`). The engine
/// keeps only its latest events, so older ones drop off the front.
pub fn new_events<'a, T: PartialEq>(current: &'a [T], seen: usize, last: Option<&T>) -> &'a [T] {
    let Some(last) = last else { return current };
    if seen > 0 && seen <= current.len() && current[seen - 1] == *last {
        return &current[seen..];
    }
    match current.iter().rposition(|e| e == last) {
        Some(i) => &current[i + 1..],
        None => current,
    }
}

/// `daemon`: syncs until stopped, answering `status`, `sync`, `pause` and `resume` like the
/// desktop client, and logging activity to stderr (JSON lines on stdout with `--json`).
pub fn daemon(ctx: &Ctx) -> Result<()> {
    let dir = ctx.dir().ok_or_else(|| CliError::error("no settings folder"))?;
    if control::is_running(&dir) {
        return Err(CliError::conflict("The desktop client or another daemon is already running for these settings."));
    }
    let engine = start(ctx)?;
    let slot = Arc::new(Mutex::new(Some(Attached {
        progress: Arc::clone(&engine.handle.progress),
        commands: engine.handle.sender(),
        server: engine.settings.server.clone(),
        local_folder: engine.settings.local_folder.clone(),
        plan: SyncPlan::from(&engine.settings),
    })));
    if let Err(e) = craftcloud_native::url_scheme::register() {
        log::warn!("couldn't register craftcloud:// links: {e}");
    }
    let _server = control::serve(&dir, slot).map_err(CliError::error)?;
    let say = |event: serde_json::Value, text: String| {
        if ctx.json {
            let _ = print_json(&event);
        } else {
            eprintln!("{text}");
        }
    };
    say(
        json!({"event": "started", "local_folder": engine.settings.local_folder}),
        format!("Syncing {} (Ctrl+C stops)", engine.settings.local_folder),
    );
    let (mut seen, mut last, mut state) = (0usize, None::<ActivityEvent>, String::new());
    loop {
        let p = progress(&engine.handle);
        for event in new_events(&p.activity, seen, last.as_ref()) {
            let a = Activity::from(event);
            say(json!({"event": "activity", "activity": a}), a.line());
        }
        seen = p.activity.len();
        last = p.activity.last().cloned();
        let now = control::state_name(&p.state).to_owned();
        if now != state {
            let status = Status::from_progress(&p, &engine.settings.server, &engine.settings.local_folder);
            say(json!({"event": "state", "state": now, "error": status.error}), format!("[{}]", super::account::state_words(&status)));
            if now == "signed_out" {
                return Err(CliError::new(Fail::SignedOut, craftcloud_native::session::SIGNED_OUT));
            }
            state = now;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::new_events;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn only_new_activity_is_logged() {
        let a = lines(&["one", "two", "three"]);
        let s = |t: &str| t.to_owned();
        assert_eq!(new_events(&a, 0, None), &a[..]);
        assert_eq!(new_events(&a, 2, Some(&s("two"))), &a[2..]);
        assert!(new_events(&a, 3, Some(&s("three"))).is_empty());
        // The front dropped off: find the last one seen.
        let b = lines(&["three", "four"]);
        assert_eq!(new_events(&b, 3, Some(&s("three"))), &b[1..]);
        assert_eq!(new_events(&b, 3, Some(&s("gone"))), &b[..], "lost track: log what's there");
    }
}
