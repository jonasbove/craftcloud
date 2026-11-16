//! The Craft apps' side: `apps`, `files`, `save`, and the `--app` filter on `ls`, `recent`,
//! `search` and `starred`. This is what the ArtCraft Launcher and the Craft apps call, so they
//! never need the user's sign-in (docs/CRAFT-INTEGRATION.md, section 3).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use craftcloud_core::api::resources::{Resource, is_internal};
use craftcloud_core::catalog::{self, APPS, CraftApp};
use craftcloud_core::sync::{SyncPlan, in_plan, local_relative};
use craftcloud_native::control::{self, Request, SendError};
use craftcloud_native::local_files::LocalView;
use serde_json::json;

use super::account;
use super::files::{self, print_items};
use super::output::{CliError, Fail, Item, Result, print_json};
use super::remote::{Ctx, Remote, clean, join, split};

/// How long `save --wait` waits for the upload.
const SAVE_WAIT: Duration = Duration::from_secs(30 * 60);

/// `--app <id>` (any case) as an app; an unknown id is a usage error that lists the ids.
pub fn pick_app(id: Option<&str>) -> Result<Option<&'static CraftApp>> {
    let Some(id) = id else { return Ok(None) };
    catalog::app(id)
        .map(Some)
        .ok_or_else(|| CliError::usage(format!("--app {id}: use one of {}", APPS.iter().map(|a| a.id).collect::<Vec<_>>().join(", "))))
}

/// `apps`: the Craft apps and the extensions each opens (built in, no network).
pub fn apps(ctx: &Ctx) -> Result<()> {
    if ctx.json {
        let list: Vec<serde_json::Value> = APPS.iter().map(app_json).collect();
        return print_json(&list);
    }
    for a in APPS {
        let experimental = if a.experimental { " (experimental in the browser)" } else { "" };
        println!("{:<14} {:<14} {}{experimental}", a.id, a.name, a.opens.join(" "));
    }
    Ok(())
}

/// One app in `apps --json`.
pub fn app_json(a: &CraftApp) -> serde_json::Value {
    json!({"id": a.id, "name": a.name, "opens": a.opens, "default": a.default, "experimental": a.experimental})
}

/// Keeps the files `app` opens (folders go; no app keeps everything) and adds where each file is
/// on this computer.
pub fn for_app(ctx: &Ctx, items: Vec<Item>, app: Option<&CraftApp>) -> Vec<Item> {
    let mut items = keep_for(items, app);
    add_local(ctx, &mut items);
    items
}

/// The files `app` opens, by extension in any case; without an app, everything.
pub fn keep_for(items: Vec<Item>, app: Option<&CraftApp>) -> Vec<Item> {
    match app {
        None => items,
        Some(a) => items.into_iter().filter(|i| i.kind == "file" && a.opens_name(&i.name)).collect(),
    }
}

/// Fills `local_path` and `sync` of files from the sync folder on this computer (when it syncs
/// this server).
fn add_local(ctx: &Ctx, items: &mut [Item]) {
    let view = if ctx.uses_clients_server() { LocalView::new(&SyncPlan::from(&ctx.settings()), Default::default()) } else { None };
    locate_all(view.as_ref(), items);
}

/// Without a view (no sync folder, or another server) files aren't here: `local_path` is `null`
/// and `sync` unknown.
fn locate_all(view: Option<&LocalView>, items: &mut [Item]) {
    for item in items.iter_mut().filter(|i| i.kind == "file") {
        let Some(view) = view else {
            item.local_path = Some(None);
            continue;
        };
        let (local, word) = view.locate(&item.path);
        item.local_path = Some(local.map(|p| p.display().to_string()));
        item.sync = Some(word.as_str().to_owned());
    }
}

/// `files [--app] [--in]`: the synced files here, from the running desktop client or daemon (its
/// local view: fast, offline); with none running, from the server, folder by folder.
pub fn files(ctx: &Ctx, app: Option<&str>, within: Option<&str>) -> Result<()> {
    let app = pick_app(app)?;
    let within = clean(within.unwrap_or(""))?;
    let extensions: Vec<String> = app.map(|a| a.opens.iter().map(|e| (*e).to_owned()).collect()).unwrap_or_default();
    let from_client = match ctx.dir().filter(|_| ctx.uses_clients_server()) {
        Some(dir) => match control::files(&dir, &extensions) {
            Ok(found) => Some(found),
            Err(SendError::NotRunning) => None,
            Err(SendError::Failed(why)) => {
                log::info!("the desktop client didn't list files ({why}); asking the server");
                None
            }
        },
        None => None,
    };
    let items: Vec<Item> = match from_client {
        Some(found) => found.into_iter().filter(|f| inside(&f.path, &within)).map(Item::local).collect(),
        None => {
            let mut remote = ctx.remote()?;
            let mut items = walk(&mut remote, &within, app)?;
            add_local(ctx, &mut items);
            items
        }
    };
    print_items(ctx, &items, |i| i.path.clone())
}

/// Whether a cloud path is `folder` or inside it (`""` is the top).
fn inside(path: &str, folder: &str) -> bool {
    folder.is_empty() || path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// Every file below a cloud folder that `app` opens, by path (CraftCloud's own `.craftcloud`
/// left out). One listing per folder: the server has no recursive listing.
fn walk(remote: &mut Remote, within: &str, app: Option<&CraftApp>) -> Result<Vec<Item>> {
    let start = remote.resolve(within)?;
    let opens = |name: &str| app.is_none_or(|a| a.opens_name(name));
    let mut out = Vec::new();
    let mut stack = match start {
        Resource::File(f) => {
            let file = Resource::File(f);
            if opens(file.name()) {
                out.push(Item::new(&file, within.to_owned()));
            }
            return Ok(out);
        }
        Resource::Folder(f) => vec![(f.id, within.to_owned())],
    };
    while let Some((id, path)) = stack.pop() {
        for r in remote.children(&id)? {
            let child = join(&path, r.name());
            if is_internal(&child) {
                continue;
            }
            match &r {
                Resource::Folder(f) => stack.push((f.id.clone(), child)),
                Resource::File(_) if opens(r.name()) => out.push(Item::new(&r, child)),
                Resource::File(_) => {}
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Where `save` puts a file: into `--to` (a cloud folder), else into the folder named after the
/// app (`--app`, or the app that opens the file by default): `WordCraft/notes.md`.
pub fn save_target(name: &str, to: Option<&str>, app: Option<&CraftApp>) -> Result<String> {
    let folder = match to {
        Some(to) => clean(to)?,
        None => {
            let app = app
                .or_else(|| catalog::for_name(name))
                .ok_or_else(|| CliError::usage(format!("no Craft app opens {name} by default: say where it goes with --to FOLDER (or --app APP)")))?;
            app.name.to_owned()
        }
    };
    Ok(join(&folder, name))
}

/// What `save` options ask for.
pub struct SaveArgs<'a> {
    pub local: &'a str,
    pub to: Option<&'a str>,
    pub app: Option<&'a str>,
    pub force: bool,
    pub wait: bool,
}

/// `save <file> [--to] [--app] [--force] [--wait]`: into the sync folder for the running client
/// to upload; with no client running (or a folder it doesn't sync), straight to the server.
pub fn save(ctx: &Ctx, args: &SaveArgs) -> Result<()> {
    let file = Path::new(args.local);
    let meta = std::fs::metadata(file).map_err(|e| CliError::not_found(format!("{}: {e}", args.local)))?;
    if !meta.is_file() {
        return Err(CliError::usage(format!("{} is not a file", args.local)));
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| CliError::usage("no file name"))?;
    let path = save_target(&name, args.to, pick_app(args.app)?)?;

    if let Some((dir, dest)) = client_target(ctx, &path) {
        let replaced = place(file, &dest, &path, args.force)?;
        let mut state = "queued";
        if args.wait {
            wait_uploaded(&dir, &path)?;
            state = "synced";
        } else {
            // Let the client look now rather than at its next scan.
            let _ = control::send(&dir, &Request::Sync { wait: false });
        }
        let local = dest.display().to_string();
        let said = if args.wait { format!("Saved {path} (uploaded)") } else { format!("Saved {path} (the desktop client uploads it)") };
        return done(
            ctx,
            json!({"path": path, "local_path": local, "via": "client", "state": state, "replaced": replaced, "size": meta.len()}),
            &said,
        );
    }

    let mut remote = ctx.remote()?;
    let folder = split(&path).0.to_owned();
    if !folder.is_empty() {
        files::make_folder(&mut remote, &folder, true)?;
    }
    let existing = remote.stat(&path)?;
    let replaced = files::upload_to(&mut remote, file, &path, existing, args.force)?;
    let said = format!("{} {path}", if replaced { "Replaced" } else { "Uploaded" });
    done(ctx, json!({"path": path, "local_path": null, "via": "server", "state": "uploaded", "replaced": replaced, "size": meta.len()}), &said)
}

fn done(ctx: &Ctx, value: serde_json::Value, human: &str) -> Result<()> {
    if ctx.json {
        print_json(&value)
    } else {
        println!("{human}");
        Ok(())
    }
}

/// The running client's settings folder and the local file a cloud path maps to, when a client
/// syncs this server and the path is in a folder it syncs.
fn client_target(ctx: &Ctx, path: &str) -> Option<(PathBuf, PathBuf)> {
    let dir = ctx.dir().filter(|_| ctx.uses_clients_server())?;
    let status = control::send(&dir, &Request::Status).ok().filter(|s| s.syncing && !s.local_folder.is_empty())?;
    let plan = SyncPlan { local_folder: status.local_folder.clone(), ..SyncPlan::from(&ctx.settings()) };
    in_plan(&plan, path, false).then_some(())?;
    Some((dir, Path::new(&status.local_folder).join(local_relative(path)?)))
}

/// Copies `src` to `dest` in the sync folder: whole or not at all (through a temporary name the
/// sync ignores). Refuses to replace a file without `force`; the client's upload of a replaced
/// file keeps the old version, as every overwrite does. Whether it replaced one.
pub fn place(src: &Path, dest: &Path, path: &str, force: bool) -> Result<bool> {
    if dest.is_dir() {
        return Err(CliError::conflict(format!("{path} is a folder")));
    }
    let existed = dest.exists();
    if existed && same_file(src, dest) {
        return Ok(false);
    }
    if existed && !force {
        return Err(CliError::conflict(format!("{path} is already in CraftCloud (--force replaces it; the old version is kept)")));
    }
    let parent = dest.parent().ok_or_else(|| CliError::error(format!("no folder for {}", dest.display())))?;
    std::fs::create_dir_all(parent).map_err(|e| CliError::error(format!("can't make {}: {e}", parent.display())))?;
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let part = parent.join(format!(".{name}.craftcloud-part"));
    let copied = std::fs::copy(src, &part).and_then(|_| std::fs::rename(&part, dest));
    copied.map_err(|e| {
        let _ = std::fs::remove_file(&part);
        CliError::error(format!("can't write {}: {e}", dest.display()))
    })?;
    Ok(existed)
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Waits until the running client has uploaded `path`: asks for syncs (a file written moments
/// ago waits a cycle, see sync.rs) and reads the file's state from the client's local view.
fn wait_uploaded(dir: &Path, path: &str) -> Result<()> {
    let extensions: Vec<String> = catalog::extension(path).into_iter().collect();
    let end = Instant::now() + SAVE_WAIT;
    loop {
        let status = control::send(dir, &Request::Sync { wait: true }).map_err(|e| match e {
            SendError::NotRunning => CliError::new(Fail::NoClient, "the desktop client stopped before the upload"),
            SendError::Failed(why) => CliError::error(why),
        })?;
        account::failed(&status)?;
        if status.state == "paused" {
            return Err(CliError::error(format!("{path} is in the sync folder, but syncing is paused (`craftcloud resume`)")));
        }
        let found = control::files(dir, &extensions).map_err(|e| CliError::error(format!("{e:?}")))?;
        match found.iter().find(|f| f.path == path).map(|f| f.sync.as_str()) {
            Some("synced") => return Ok(()),
            Some("error") => return Err(CliError::error(format!("{path} couldn't be uploaded; see `craftcloud status`"))),
            Some("conflict") => return Err(CliError::conflict(format!("{path} changed in the cloud too: both versions were kept"))),
            None => return Err(CliError::not_found(format!("{path} is no longer in the sync folder"))),
            Some(_) => {}
        }
        if Instant::now() > end {
            return Err(CliError::error(format!("{path} is still uploading")));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::{inside, keep_for, locate_all, pick_app, place, save_target};
    use crate::cli::output::Item;
    use craftcloud_core::sync::SyncPlan;
    use craftcloud_native::local_files::LocalView;

    fn file(path: &str, kind: &'static str) -> Item {
        Item {
            path: path.into(),
            name: path.rsplit('/').next().unwrap().into(),
            kind,
            size: Some(1),
            modified: "2026-11-13T21:54:00Z".into(),
            starred: false,
            id: "f".into(),
            local_path: None,
            sync: None,
            starred_at: None,
            opened_at: None,
            trashed_at: None,
            expires_at: None,
            score: None,
        }
    }

    #[test]
    fn an_app_keeps_the_files_it_opens() {
        let items = vec![
            file("Documents/Q3 report.DOCX", "file"),
            file("Documents/notes.md", "file"),
            file("Documents/todo.TXT", "file"),
            file("Documents/budget.xlsx", "file"),
            file("Documents/old.md", "folder"),
            file("README", "file"),
        ];
        let word = pick_app(Some("WordCraft")).unwrap();
        let kept: Vec<String> = keep_for(items.clone(), word).into_iter().map(|i| i.path).collect();
        assert_eq!(kept, ["Documents/Q3 report.DOCX", "Documents/notes.md", "Documents/todo.TXT"], "folders go, any case matches");
        assert_eq!(keep_for(items.clone(), None).len(), 6, "no --app: everything");
        let grid = pick_app(Some("gridcraft")).unwrap();
        assert_eq!(keep_for(items, grid).len(), 1);
        let err = pick_app(Some("paintcraft")).unwrap_err();
        assert_eq!(err.kind.code(), 2);
        assert!(err.message.contains("wordcraft"), "{}", err.message);
    }

    #[test]
    fn saves_go_to_the_apps_folder_or_where_asked() {
        assert_eq!(save_target("notes.md", None, None).unwrap(), "WordCraft/notes.md");
        assert_eq!(save_target("Q3.XLSX", None, None).unwrap(), "GridCraft/Q3.XLSX");
        assert_eq!(save_target("logo.png", None, pick_app(Some("figcraft")).unwrap()).unwrap(), "FigCraft/logo.png");
        assert_eq!(save_target("notes.md", Some("/Documents/Q3/"), None).unwrap(), "Documents/Q3/notes.md");
        assert_eq!(save_target("notes.md", Some("/"), None).unwrap(), "notes.md");
        assert_eq!(save_target("archive.zip", None, None).unwrap_err().kind.code(), 2, "no app: --to is needed");
        assert!(save_target("a.md", Some("../x"), None).is_err());
        assert!(inside("Documents/a.md", "Documents") && inside("a.md", "") && !inside("Documents2/a.md", "Documents"));
    }

    #[test]
    fn a_save_lands_whole_in_the_sync_folder_and_never_overwrites_by_accident() {
        let sync = tempfile::tempdir().unwrap();
        let src_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("notes.md");
        std::fs::write(&src, "# v1").unwrap();
        let path = save_target("notes.md", None, None).unwrap();
        // Joined part by part: on Windows the path the listing gives back uses `\`.
        let dest = sync.path().join("WordCraft").join("notes.md");

        assert!(!place(&src, &dest, &path, false).unwrap(), "a new file");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# v1");
        let left: Vec<_> = std::fs::read_dir(sync.path().join("WordCraft")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, ["notes.md"], "no temporary file left behind");

        std::fs::write(&src, "# v2").unwrap();
        assert_eq!(place(&src, &dest, &path, false).unwrap_err().kind.code(), 5, "there already: --force needed");
        assert!(place(&src, &dest, &path, true).unwrap(), "replaced");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# v2");
        assert!(!place(&dest, &dest, &path, false).unwrap(), "saving the synced copy itself is fine");

        // The listing commands now see it here, waiting for the client's upload.
        let plan = SyncPlan { local_folder: sync.path().display().to_string(), ..SyncPlan::default() };
        let view = LocalView::new(&plan, Default::default()).unwrap();
        let mut items = vec![file("WordCraft/notes.md", "file"), file("Elsewhere/a.md", "file")];
        locate_all(Some(&view), &mut items);
        assert_eq!(items[0].local_path, Some(Some(dest.display().to_string())));
        assert_eq!(items[0].sync.as_deref(), Some("pending"));
        assert_eq!((items[1].local_path.clone(), items[1].sync.as_deref()), (Some(None), Some("pending")));

        // No sync folder: the path is there as null, the state is left out.
        let mut items = vec![file("WordCraft/notes.md", "file")];
        locate_all(None, &mut items);
        let json = serde_json::to_value(&items[0]).unwrap();
        assert!(json.as_object().unwrap().contains_key("local_path") && json["local_path"].is_null());
        assert!(json.get("sync").is_none());
    }
}
