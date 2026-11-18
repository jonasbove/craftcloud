//! Syncing without the desktop client: `sync --once` (one full pass, then exit) and `daemon` (the
//! sync engine without a window or tray, until stopped). Both use the saved settings and sign-in,
//! and claim the sync folder first, so only one engine ever syncs a folder.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craftcloud_core::sync::{ActivityEvent, ClientSettings, SyncPlan, SyncProgress, SyncState};
use craftcloud_native::control::{self, Activity, Attached, FolderLock, Status};
use craftcloud_native::storage::probe::Limits;
use craftcloud_native::storage::webdav::WebDav;
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
    let device = ctx.device();
    let handle = if ctx.is_webdav() {
        // The driver connects on the sync thread (the check on connect may take a while).
        let session = ctx.app_password_session()?;
        let dir = ctx.dir();
        let device_too = device.clone();
        let connect: craftcloud_native::sync::Connect =
            Box::new(move || Ok(WebDav::connect(session, &device_too, dir.as_deref(), Limits::default())));
        craftcloud_native::sync::start_webdav(SyncPlan::from(&settings), connect, device, Arc::new(|| {}))
    } else {
        let remote = ctx.remote()?;
        let mut plan = SyncPlan::from(&settings);
        plan.server = remote.server().to_string();
        craftcloud_native::sync::start_with(plan, remote.session, device, Arc::new(|| {}))
    };
    Ok(Engine { handle, _lock: lock, settings })
}

fn progress(handle: &Handle) -> SyncProgress {
    handle.progress.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Runs sync pass number `ticket` and waits until it's done (or `deadline`).
fn pass(handle: &Handle, ticket: u64, deadline: Instant) -> SyncProgress {
    handle.send(Command::SyncFor(ticket));
    loop {
        let p = progress(handle);
        if p.served >= ticket || Instant::now() > deadline {
            return p;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// How long to wait before a second pass: when the first ended up to date but held files that
/// were being saved, until they count as settled. `None`: no second pass.
fn retry_after(p: &SyncProgress) -> Option<Duration> {
    let settle = Duration::from_millis(craftcloud_native::sync::SETTLE_MS.unsigned_abs() + 500);
    (p.state == SyncState::UpToDate && !p.waiting.is_empty()).then_some(settle)
}

/// `sync --once` (and `sync` when no desktop client runs): one full pass.
pub fn once(ctx: &Ctx, deletes: super::account::Deletes) -> Result<()> {
    let engine = start(ctx)?;
    match deletes {
        super::account::Deletes::Hold => {}
        super::account::Deletes::Confirm => engine.handle.send(Command::ConfirmDeletes),
        super::account::Deletes::Discard => engine.handle.send(Command::DiscardDeletes),
    }
    let deadline = Instant::now() + Duration::from_secs(6 * 60 * 60);
    let mut p = pass(&engine.handle, 1, deadline);
    // A file written moments before was held for the next pass (an app may still be saving it):
    // wait that out once and sync it, rather than say "up to date" without it (#15). Only once,
    // so an app that keeps writing can't keep this running; what is still held is reported.
    if let Some(wait) = retry_after(&p) {
        std::thread::sleep(wait);
        p = pass(&engine.handle, 2, deadline);
    }
    drop(engine.handle);
    let status = Status::from_progress(&p, &engine.settings.server, &engine.settings.local_folder);
    let activity: Vec<Activity> = p.activity.iter().map(Activity::from).collect();
    if ctx.json {
        print_json(&json!({
            "state": status.state,
            "mode": status.mode,
            "error": status.error,
            "pending_deletes": status.pending_deletes,
            "files_done": p.files_done,
            "files_total": p.files_total,
            "conflicts": status.conflicts,
            "not_synced": status.not_synced,
            "waiting": status.waiting,
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
    use super::{new_events, retry_after};
    use craftcloud_core::sync::{SyncProgress, SyncState};

    /// #15: a pass that held a file being saved is followed by one more, after the settle time,
    /// instead of ending "up to date" without it.
    #[test]
    fn a_held_file_gets_a_second_pass() {
        let mut p = SyncProgress { state: SyncState::UpToDate, ..SyncProgress::default() };
        assert_eq!(retry_after(&p), None, "nothing held: one pass");
        p.waiting = vec!["Documents/notes.md".into()];
        let wait = retry_after(&p).expect("a second pass");
        assert!(wait.as_millis() > craftcloud_native::sync::SETTLE_MS.unsigned_abs().into(), "after the file has settled");
        p.state = SyncState::Offline("down".into());
        assert_eq!(retry_after(&p), None, "a pass that didn't finish isn't repeated");
    }

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
