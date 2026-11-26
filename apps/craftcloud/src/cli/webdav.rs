//! The file commands on another kind of server, through the storage trait (docs/WEBDAV-STORAGE.md
//! § 11): `ls`, `get`, `put`, `mkdir`, `mv`, `rm`, `files`, the direct `save`, and the client
//! trash and version commands (`trash ls|restore|rm|empty`, `versions`). CraftCloud's own server
//! keeps its REST paths (ids, stars, the server's trash); these go through `RemoteStorage` only,
//! so every write carries its guard and a delete goes to the client's trash by `MOVE`. What
//! CraftCloud's own server has and this kind hasn't (share links, stars, search, people, the
//! server's settings) answers "not available on this server" (exit code 10).

use std::io::Write as _;
use std::path::Path;

use craftcloud_core::api::dav;
use craftcloud_core::storage::Trash;
use craftcloud_native::storage::{Entry, MoveGuard, Moved, OwnPath, RemoteStorage, Restored, StorageError, TrashGuard, TrashItem, Trashed, Written};
use serde_json::json;

use super::TrashCmd;
use super::files::{Overwrite, Plan, Saved, confirm, plan_write, print_items};
use super::output::{CliError, Item, Result, http_etag, print_json};
use super::remote::{Ctx, clean, join, split};

/// A storage error as a command-line error: not found is 4, no room and a refusal 1 with the
/// server's words, a transport error or a sign-out as `CliError::transport` reads them.
fn cli(e: StorageError) -> CliError {
    match e {
        StorageError::NotFound(why) => CliError::not_found(why),
        StorageError::Failed(why) => CliError::transport(why),
        other => CliError::error(other.to_string()),
    }
}

/// A listing entry as an item (no id, no star: this kind of server has neither).
fn item(e: &Entry) -> Item {
    let modified = e.modified.as_deref().and_then(|m| jiff::fmt::rfc2822::parse(m).ok()).map(|z| z.timestamp().to_string()).unwrap_or_default();
    Item {
        path: e.path.clone(),
        name: e.path.rsplit('/').next().unwrap_or(&e.path).to_owned(),
        kind: if e.is_folder { "folder" } else { "file" },
        size: (!e.is_folder).then_some(e.size),
        modified,
        starred: false,
        id: String::new(),
        local_path: None,
        sync: None,
        starred_at: None,
        opened_at: None,
        trashed_at: None,
        expires_at: None,
        score: None,
        etag: if e.is_folder { None } else { e.etag.as_deref().and_then(http_etag) },
    }
}

/// What is at `path`: `None` when nothing is (the top is a folder).
fn stat(storage: &mut dyn RemoteStorage, path: &str) -> Result<Option<Entry>> {
    if path.is_empty() {
        return Ok(Some(Entry { path: String::new(), is_folder: true, etag: None, size: 0, modified: None, file_id: None, checksum: None }));
    }
    let (parent, _) = split(path);
    match storage.list(parent) {
        Ok(entries) => Ok(entries.into_iter().find(|e| e.path == path)),
        Err(StorageError::NotFound(_)) => Ok(None),
        Err(e) => Err(cli(e)),
    }
}

fn done(ctx: &Ctx, value: serde_json::Value, human: &str) -> Result<()> {
    if ctx.json {
        print_json(&value)
    } else {
        if !human.is_empty() {
            println!("{human}");
        }
        Ok(())
    }
}

/// `ls [path]`.
pub fn ls(ctx: &Ctx, path: &str, all: bool, app: Option<&craftcloud_core::catalog::CraftApp>) -> Result<()> {
    let (mut storage, _) = ctx.webdav()?;
    let found = stat(&mut storage, path)?.ok_or_else(|| CliError::not_found(format!("{path}: no such file or folder")))?;
    let items: Vec<Item> = if found.is_folder {
        storage
            .list(path)
            .map_err(cli)?
            .iter()
            .filter(|e| all || craftcloud_core::api::resources::is_internal(path) || !craftcloud_core::api::resources::is_internal(&e.path))
            .map(item)
            .collect()
    } else {
        vec![item(&found)]
    };
    print_items(ctx, &super::craft::for_app(ctx, items, app), |i| i.name.clone())
}

/// `get <remote> [local]`: a file (a folder has no zip here).
pub fn get(ctx: &Ctx, path: &str, local: Option<&str>, force: bool) -> Result<()> {
    let (mut storage, _) = ctx.webdav()?;
    let entry = stat(&mut storage, path)?.ok_or_else(|| CliError::not_found(format!("{path}: no such file or folder")))?;
    if entry.is_folder {
        return Err(CliError::unsupported("downloading a folder as a zip"));
    }
    let mut resp = storage.download(path, None).map_err(cli)?;
    if !(200..300).contains(&resp.status()) {
        return Err(CliError::error(format!("{path}: the server answered {}", resp.status())));
    }
    if local == Some("-") {
        let mut out = std::io::stdout().lock();
        resp.stream_to(&mut out).map_err(CliError::error)?;
        return out.flush().map_err(|e| CliError::error(e.to_string()));
    }
    let dest = super::files::target(local, split(path).1);
    if dest.exists() && !force {
        return Err(CliError::conflict(format!("{} is already there (--force replaces it)", dest.display())));
    }
    let part = dest.with_file_name(format!(".{}.craftcloud-part", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    let written = (|| {
        let mut file = std::fs::File::create(&part).map_err(|e| format!("can't write {}: {e}", part.display()))?;
        let n = resp.stream_to(&mut file)?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&part, &dest).map_err(|e| format!("can't write {}: {e}", dest.display()))?;
        Ok::<u64, String>(n)
    })();
    let bytes = written
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&part);
        })
        .map_err(CliError::error)?;
    done(ctx, json!({"path": path, "saved_to": dest.display().to_string(), "size": bytes}), &format!("Saved {}", dest.display()))
}

/// `put <local> [remote]`.
pub fn put(ctx: &Ctx, local: &str, remote_path: Option<&str>, force: bool) -> Result<()> {
    let file = Path::new(local);
    let meta = std::fs::metadata(file).map_err(|e| CliError::not_found(format!("{local}: {e}")))?;
    if !meta.is_file() {
        return Err(CliError::usage(format!("{local} is not a file (upload folders file by file, or let the desktop client sync them)")));
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| CliError::usage("no file name"))?;
    let (mut storage, _) = ctx.webdav_for_writes()?;
    let wanted = remote_path.unwrap_or("");
    let mut path = clean(wanted)?;
    let mut existing = stat(&mut storage, &path)?;
    if wanted.is_empty() || wanted.ends_with('/') || existing.as_ref().is_some_and(|e| e.is_folder) {
        path = join(&path, &name);
        existing = stat(&mut storage, &path)?;
    }
    let saved = upload_to(&mut storage, file, &path, existing, &Overwrite::from_force(force))?;
    let verb = if saved.replaced { "Replaced" } else { "Uploaded" };
    done(ctx, json!({"path": path, "size": meta.len(), "replaced": saved.replaced, "etag": saved.etag}), &format!("{verb} {path}"))
}

/// Uploads `file` to `path` (`existing` is what is there now): refuses to replace a file unless
/// `overwrite` allows it, and keeps the old version first when it does.
pub fn upload_to(storage: &mut dyn RemoteStorage, file: &Path, path: &str, existing: Option<Entry>, overwrite: &Overwrite) -> Result<Saved> {
    let written = match plan_write(path, existing.as_ref().map(|e| (e.is_folder, e.etag.as_deref())), overwrite)? {
        Plan::Create => storage.create(path, file),
        Plan::Replace(etag) => {
            let etag = etag.ok_or_else(|| {
                CliError::conflict(format!("{path} is listed without a version tag, so it can't be shown unchanged; nothing was written"))
            })?;
            storage.keep_version(path).map_err(cli)?;
            storage.replace(path, &etag, file)
        }
    };
    match written {
        Err(StorageError::NotFound(_)) => {
            Err(CliError::not_found(format!("the folder {} doesn't exist (make it with `craftcloud mkdir -p`)", split(path).0)))
        }
        Err(e) => Err(cli(e).context(path)),
        Ok(Written::Conflict) => Err(CliError::conflict(format!("{path} changed on the server meanwhile; nothing was written"))),
        Ok(Written::Saved { etag }) => Ok(Saved { replaced: existing.is_some(), etag: http_etag(&etag) }),
    }
}

/// `mkdir [-p] <path>`.
pub fn mkdir(ctx: &Ctx, path: &str, parents: bool) -> Result<()> {
    let path = clean(path)?;
    if path.is_empty() {
        return Err(CliError::usage("which folder?"));
    }
    let (mut storage, _) = ctx.webdav_for_writes()?;
    let created = make_folder(&mut storage, &path, parents)?;
    let said = if created { format!("Made {path}") } else { format!("{path} was already there") };
    done(ctx, json!({"path": path, "created": created}), &said)
}

/// Makes a folder (with `parents`: its missing parents too, and an existing one is fine).
/// Whether it made one.
pub fn make_folder(storage: &mut dyn RemoteStorage, path: &str, parents: bool) -> Result<bool> {
    let parts: Vec<&str> = path.split('/').collect();
    let steps: Vec<String> = if parents { (1..=parts.len()).map(|n| parts[..n].join("/")).collect() } else { vec![path.to_owned()] };
    let mut created = false;
    for step in steps {
        match stat(storage, &step)? {
            Some(e) if e.is_folder && parents => continue,
            Some(_) => return Err(CliError::conflict(format!("{step} is already there"))),
            None => {}
        }
        match storage.make_folder(&step) {
            Ok(()) => created = true,
            Err(StorageError::Status(409 | 404)) => {
                return Err(CliError::not_found(format!("the folder {} doesn't exist (-p makes it)", split(&step).0)));
            }
            Err(e) => return Err(cli(e).context(&step)),
        }
    }
    Ok(created)
}

/// `mv`: into a folder when the destination is one, else to that name. Never onto an existing
/// name, and a file only while it is still the version last listed.
pub fn mv(ctx: &Ctx, from: &str, to: &str) -> Result<()> {
    let (from, to_clean) = (clean(from)?, clean(to)?);
    if from.is_empty() {
        return Err(CliError::usage("the top of the cloud can't be moved"));
    }
    let (mut storage, _) = ctx.webdav_for_writes()?;
    let source = stat(&mut storage, &from)?.ok_or_else(|| CliError::not_found(format!("{from}: no such file or folder")))?;
    let into = to.ends_with('/') || to_clean.is_empty() || stat(&mut storage, &to_clean)?.is_some_and(|e| e.is_folder);
    let dest = if into { join(&to_clean, split(&from).1) } else { to_clean };
    if dest == from || dest.starts_with(&format!("{from}/")) {
        return Err(CliError::usage(format!("{from} can't go into itself")));
    }
    let guard = if source.is_folder {
        MoveGuard::Folder
    } else {
        MoveGuard::File { etag: source.etag.ok_or_else(|| CliError::conflict(format!("{from} is listed without a version tag; nothing was moved")))? }
    };
    match storage.move_to(&from, &dest, guard).map_err(cli)? {
        Moved::Done => {}
        Moved::Changed => return Err(CliError::conflict(format!("{from} changed on the server meanwhile; nothing was moved"))),
        Moved::Unversioned => return Err(CliError::conflict(format!("{from} is listed without a version tag; nothing was moved"))),
        Moved::Refused(412) => return Err(CliError::conflict(format!("{dest} is already there"))),
        Moved::Refused(409 | 404) => return Err(CliError::not_found(format!("the folder {} doesn't exist", split(&dest).0))),
        Moved::Refused(status) => return Err(CliError::error(format!("the server answered {status}"))),
    }
    // The kept versions go along.
    storage.move_versions(&from, &dest).map_err(cli)?;
    done(ctx, json!({"from": from, "to": dest}), &format!("Moved {from} to {dest}"))
}

/// Every file below a folder, every level, as (path, ETag): what a folder trash must still find.
fn files_below(storage: &mut dyn RemoteStorage, folder: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut level = vec![folder.to_owned()];
    while let Some(f) = level.pop() {
        for e in storage.list(&f).map_err(cli)? {
            if e.is_folder {
                level.push(e.path);
            } else {
                out.push((e.path, e.etag.unwrap_or_default()));
            }
        }
    }
    Ok(out)
}

/// `rm <path>…`: to CraftCloud's trash on the server (`.craftcloud/trash/`), by one `MOVE` each,
/// only while unchanged since listed.
pub fn rm(ctx: &Ctx, paths: &[String]) -> Result<()> {
    let (mut storage, _) = ctx.webdav_for_writes()?;
    let mut trashed = Vec::new();
    for p in paths {
        let path = clean(p)?;
        if path.is_empty() {
            return Err(CliError::usage("the top of the cloud can't go to the trash"));
        }
        let found = stat(&mut storage, &path)?.ok_or_else(|| CliError::not_found(format!("{path}: no such file or folder")))?;
        let guard = if found.is_folder {
            TrashGuard::Folder { files: files_below(&mut storage, &path)? }
        } else {
            TrashGuard::File { etag: found.etag.ok_or_else(|| CliError::conflict(format!("{path} is listed without a version tag; it stays")))? }
        };
        match storage.trash(&path, guard).map_err(|e| cli(e).context(&path))? {
            Trashed::Done => {}
            Trashed::Changed => return Err(CliError::conflict(format!("{path} changed on the server meanwhile; nothing was moved"))),
            Trashed::Unversioned => return Err(CliError::conflict(format!("{path} is listed without a version tag; it stays"))),
        }
        if !ctx.json {
            println!("Moved {path} to the trash");
        }
        trashed.push(path);
    }
    if ctx.json { print_json(&json!({"trashed": trashed})) } else { Ok(()) }
}

/// Every file below `within` that `app` opens, every level.
pub fn walk(ctx: &Ctx, within: &str, app: Option<&craftcloud_core::catalog::CraftApp>) -> Result<Vec<Item>> {
    let (mut storage, _) = ctx.webdav()?;
    let start = stat(&mut storage, within)?.ok_or_else(|| CliError::not_found(format!("{within}: no such folder")))?;
    if !start.is_folder {
        return Err(CliError::usage(format!("--in {within}: that's a file")));
    }
    let mut out = Vec::new();
    let mut level = vec![within.to_owned()];
    while let Some(folder) = level.pop() {
        for e in storage.list(&folder).map_err(cli)? {
            if craftcloud_core::api::resources::is_internal(&e.path) {
                continue;
            }
            if e.is_folder {
                level.push(e.path);
            } else if app.is_none_or(|a| a.opens_name(e.path.rsplit('/').next().unwrap_or(&e.path))) {
                out.push(item(&e));
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `save` straight to the server (no client, a folder this computer doesn't sync, or
/// `--if-match`): the folder made first (not for `--if-match`: the file must be there), the old
/// version kept before an overwrite.
pub fn save_direct(ctx: &Ctx, local: &Path, path: &str, overwrite: &Overwrite) -> Result<Saved> {
    let (mut storage, _) = ctx.webdav_for_writes()?;
    let (folder, _) = split(path);
    if !folder.is_empty() && !matches!(overwrite, Overwrite::IfMatch(_)) {
        make_folder(&mut storage, folder, true)?;
    }
    let existing = stat(&mut storage, path)?;
    upload_to(&mut storage, local, path, existing, overwrite)
}

/// A trashed item as an output item: its original cloud path and when it was trashed.
fn trash_item(i: &TrashItem) -> Item {
    Item {
        path: i.path.clone(),
        name: i.path.rsplit('/').next().unwrap_or(&i.path).to_owned(),
        kind: "",
        size: None,
        modified: String::new(),
        starred: false,
        id: i.id.clone(),
        local_path: None,
        sync: None,
        starred_at: None,
        opened_at: None,
        trashed_at: Some(i.trashed_at.clone()),
        expires_at: None,
        score: None,
        etag: None,
    }
}

/// Finds one trashed item by id, by its original cloud path, or (when only one has it) by name.
fn pick<'a>(items: &'a [TrashItem], wanted: &str) -> Result<&'a TrashItem> {
    let path = wanted.trim_matches('/');
    if let Some(i) = items.iter().find(|i| i.id == wanted) {
        return Ok(i);
    }
    let by_path: Vec<&TrashItem> = items.iter().filter(|i| i.path == path).collect();
    let matches = if by_path.is_empty() { items.iter().filter(|i| i.path.rsplit('/').next() == Some(path)).collect() } else { by_path };
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(CliError::not_found(format!("{wanted} isn't in the trash"))),
        more => Err(CliError::conflict(format!(
            "{} items in the trash match {wanted}; use the id from `craftcloud trash ls --json` ({})",
            more.len(),
            more.iter().map(|i| i.id.clone()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// The trash entry folder (`.craftcloud/trash/<time>-<device>`) an item lives in, from its id. An
/// entry holds exactly one trashed item (the driver makes a fresh entry for every `MOVE` into the
/// trash), so removing the entry removes that item and nothing else.
fn entry_folder(id: &str) -> Option<OwnPath> {
    let entry = id.strip_prefix(dav::TRASH_DIR)?.trim_start_matches('/').split('/').next().filter(|e| !e.is_empty())?;
    OwnPath::new(&format!("{}/{entry}", dav::TRASH_DIR))
}

/// `trash ls|restore|rm|empty` on another WebDAV server, through the storage trait
/// (docs/WEBDAV-STORAGE.md § 11): the server's own trash where the check on connect confirmed it
/// (Nextcloud), else CraftCloud's own in `.craftcloud/trash/` on the server. `ls` reads; the rest
/// write, so they need two-way sync (`webdav_for_writes`). `rm` and `empty` delete for good, on the
/// person's own command: CraftCloud's trash through `purge_own`, a server's own through its
/// profile (§ 16; never by the engine, the check on connect or retention).
pub fn trash(ctx: &Ctx, cmd: TrashCmd) -> Result<()> {
    match cmd {
        TrashCmd::Ls { all } => {
            let (mut storage, _) = ctx.webdav()?;
            // As on CraftCloud's server, `--all` shows CraftCloud's own `.craftcloud` paths too.
            let items: Vec<Item> = storage
                .trash_list()
                .map_err(cli)?
                .iter()
                .filter(|i| all || !craftcloud_core::api::resources::is_internal(&i.path))
                .map(trash_item)
                .collect();
            print_items(ctx, &items, |i| i.path.clone())
        }
        TrashCmd::Restore { item, to, rename } => {
            let (mut storage, _) = ctx.webdav_for_writes()?;
            let all = storage.trash_list().map_err(cli)?;
            let found = pick(&all, &item)?.clone();
            let from = found.path.clone();
            // `--as PATH` restores elsewhere; otherwise back where it was. As on CraftCloud's
            // server, a place that is taken is refused unless `--rename`, which restores next to
            // it as "NAME (restored).EXT" (the driver's `MOVE` with `Overwrite: F` does that; it
            // never overwrites).
            let target = match to {
                Some(to) => TrashItem { path: clean(&to)?, ..found },
                None => found,
            };
            let dest = target.path.clone();
            if !rename && stat(&mut storage, &dest)?.is_some() {
                return Err(CliError::conflict(format!("{dest} is there now; use --rename to restore next to it, or --as PATH")));
            }
            storage.restore(&target).map_err(cli)?;
            let human = if dest == from { format!("Restored {from}") } else { format!("Restored {from} as {dest}") };
            done(ctx, json!({"restored": dest, "from": from, "id": target.id}), &human)
        }
        TrashCmd::Rm { item } => {
            let (mut storage, _) = ctx.webdav_for_writes()?;
            let all = storage.trash_list().map_err(cli)?;
            let found = pick(&all, &item)?.clone();
            // The server's own trash: the person asked, so the item goes from there for good
            // (docs/WEBDAV-STORAGE.md § 16), as `trash rm` does on CraftCloud's server.
            if storage.capabilities().trash == Trash::Server {
                storage.purge_server_trash_item(&found).map_err(cli)?;
                return done(ctx, json!({"deleted": found.path, "id": found.id}), &format!("Deleted {} for good", found.path));
            }
            let entry = entry_folder(&found.id).ok_or_else(|| CliError::error(format!("{}: not a trash item", found.id)))?;
            storage.purge_own(&entry).map_err(cli)?;
            done(ctx, json!({"deleted": found.path, "id": found.id}), &format!("Deleted {} for good", found.path))
        }
        TrashCmd::Empty { yes } => {
            // The mode first: on a download-only server the answer is exit 10, not a question.
            let (mut storage, _) = ctx.webdav_for_writes()?;
            if !yes {
                confirm("Delete everything in the trash for good?")?;
            }
            // The server's own trash: emptied there, on the person's word (§ 16).
            if storage.capabilities().trash == Trash::Server {
                storage.empty_server_trash().map_err(cli)?;
                return done(ctx, json!({"emptied": true}), "The trash is empty.");
            }
            let entries = match storage.list(dav::TRASH_DIR) {
                Ok(entries) => entries,
                Err(StorageError::NotFound(_)) => Vec::new(),
                Err(e) => return Err(cli(e)),
            };
            for e in entries.iter().filter(|e| e.is_folder) {
                if let Some(own) = OwnPath::new(&e.path) {
                    storage.purge_own(&own).map_err(cli)?;
                }
            }
            done(ctx, json!({"emptied": true}), "The trash is empty.")
        }
    }
}

/// `versions <path> [restore <n>]` on another WebDAV server: the server's own versions where the
/// check on connect confirmed them (Nextcloud, by file id), else the kept versions in
/// `.craftcloud/versions/<path>/`, read and restored through the same code OxiCloud uses. Listing
/// reads; a restore writes, so it needs two-way sync.
pub fn versions(ctx: &Ctx, path: &str, restore: bool, n: Option<usize>) -> Result<()> {
    let path = clean(path)?;
    if restore {
        let n = n.ok_or_else(|| CliError::usage("which version? craftcloud versions <path> restore <n> (1 = newest)"))?;
        let (mut storage, _) = ctx.webdav_for_writes()?;
        let list = storage.list_versions(&path).map_err(cli)?;
        let version = n
            .checked_sub(1)
            .and_then(|i| list.get(i))
            .ok_or_else(|| CliError::not_found(format!("{path} has no version {n} (it has {})", list.len())))?
            .clone();
        // A guarded write: a file that changed on the server meanwhile is never overwritten.
        if storage.restore_version(&path, &version.stamp).map_err(cli)? == Restored::Changed {
            return Err(CliError::conflict(format!("{path} changed on the server meanwhile; nothing was restored (the newer file stays)")));
        }
        if ctx.json {
            let mut out = super::sharing::version_json(n, &version);
            out["path"] = json!(path);
            out["restored"] = json!(version.stamp);
            return print_json(&out);
        }
        println!("Restored {path} to version {n} (the replaced one is now version 1).");
        return Ok(());
    }
    let (mut storage, _) = ctx.webdav()?;
    if stat(&mut storage, &path)?.is_none() {
        return Err(CliError::not_found(format!("{path}: no such file")));
    }
    if n.is_some() {
        return Err(CliError::usage("to restore a version: craftcloud versions <path> restore <n>"));
    }
    let list = storage.list_versions(&path).map_err(cli)?;
    if ctx.json {
        let rows: Vec<_> = list.iter().enumerate().map(|(i, v)| super::sharing::version_json(i + 1, v)).collect();
        return print_json(&rows);
    }
    if list.is_empty() {
        println!("{path} has no earlier versions.");
    }
    for (i, v) in list.iter().enumerate() {
        println!("{}", super::sharing::version_line(i + 1, v));
    }
    Ok(())
}
