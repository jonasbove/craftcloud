//! Files and folders: ls, get, put, mkdir, mv, cp, rm, trash, star, starred, recent, search.
//!
//! Bytes, folders, moves and copies go over WebDAV, by path; the trash, stars, recent and search
//! are REST, by id (found by walking the path from the root).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use craftcloud_core::api::dav::{self, Uploaded, Write};
use craftcloud_core::api::favorites;
use craftcloud_core::api::resources::{Kind, Resource, is_internal};
use craftcloud_core::api::search::{self, TypeFilter};
use craftcloud_core::api::trash::{self, TrashItem};
use craftcloud_core::api::{expect_ok, files as rest_files, folders};
use craftcloud_core::http::HttpResponse;
use serde_json::json;

use super::TrashCmd;
use super::craft;
use super::output::{CliError, Fail, Item, Result, line, print_json};
use super::remote::{Ctx, Remote, clean, join, split};

/// Prints items: JSON array, or one line each (`show` picks the name shown).
pub(super) fn print_items(ctx: &Ctx, items: &[Item], show: impl Fn(&Item) -> String) -> Result<()> {
    if ctx.json {
        return print_json(&items);
    }
    for item in items {
        println!("{}", line(item, &show(item)));
    }
    Ok(())
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

/// `ls [path] [--app]`.
pub fn ls(ctx: &Ctx, path: &str, all: bool, app: Option<&str>) -> Result<()> {
    let app = craft::pick_app(app)?;
    let path = clean(path)?;
    if ctx.is_webdav() {
        return super::webdav::ls(ctx, &path, all, app);
    }
    let mut remote = ctx.remote()?;
    let folder = match remote.resolve(&path)? {
        Resource::Folder(f) => f,
        Resource::File(f) => {
            let items = craft::for_app(ctx, vec![Item::new(&Resource::File(f), path.clone())], app);
            return print_items(ctx, &items, |i| i.name.clone());
        }
    };
    let items: Vec<Item> = remote
        .children(&folder.id)?
        .iter()
        // Listing inside `.craftcloud` on purpose shows it; elsewhere it stays out of sight.
        .filter(|r| all || is_internal(&path) || !is_internal(&join(&path, r.name())))
        .map(|r| Item::new(r, join(&path, r.name())))
        .collect();
    print_items(ctx, &craft::for_app(ctx, items, app), |i| i.name.clone())
}

/// A non-2xx streamed answer as an error.
fn refused(mut resp: ureq::http::Response<ureq::Body>) -> CliError {
    let body = resp.body_mut().read_to_vec().unwrap_or_default();
    let answer = HttpResponse::new(resp.status().as_u16(), body);
    match expect_ok(&answer) {
        Err(e) => CliError::api(&e),
        Ok(()) => CliError::error(format!("the server answered {}", answer.status)),
    }
}

/// `get <remote> [local]`.
pub fn get(ctx: &Ctx, remote_path: &str, local: Option<&str>, force: bool) -> Result<()> {
    let path = clean(remote_path)?;
    if ctx.is_webdav() {
        return super::webdav::get(ctx, &path, local, force);
    }
    let mut remote = ctx.remote()?;
    let entry = remote.stat(&path)?.ok_or_else(|| CliError::not_found(format!("{path}: no such file or folder")))?;
    let (name, req) = if entry.is_folder {
        let id = remote.resolve(&path)?.id().to_owned();
        let name = if path.is_empty() { "CraftCloud".to_owned() } else { split(&path).1.to_owned() };
        (format!("{name}.zip"), folders::download_zip(remote.server(), &id))
    } else {
        (split(&path).1.to_owned(), dav::download(remote.server(), &path))
    };
    let mut resp = remote.session.open(&req).map_err(CliError::transport)?;
    if !resp.status().is_success() {
        return Err(refused(resp).context(&path));
    }
    if local == Some("-") {
        let mut out = std::io::stdout().lock();
        craftcloud_native::http::stream_body(&mut resp, &mut out, &remote.session.cancel).map_err(CliError::error)?;
        return out.flush().map_err(|e| CliError::error(e.to_string()));
    }
    let dest = target(local, &name);
    if dest.exists() && !force {
        return Err(CliError::conflict(format!("{} is already there (--force replaces it)", dest.display())));
    }
    let part = dest.with_file_name(format!(".{}.craftcloud-part", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    let written = (|| {
        let mut file = std::fs::File::create(&part).map_err(|e| format!("can't write {}: {e}", part.display()))?;
        let n = craftcloud_native::http::stream_body(&mut resp, &mut file, &remote.session.cancel)?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&part, &dest).map_err(|e| format!("can't write {}: {e}", dest.display()))?;
        Ok::<u64, String>(n)
    })();
    let bytes = written.inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    });
    let bytes = bytes.map_err(CliError::error)?;
    done(ctx, json!({"path": path, "saved_to": dest.display().to_string(), "size": bytes}), &format!("Saved {}", dest.display()))
}

/// Where a download goes: the current folder, into a folder, or a file name.
pub(super) fn target(local: Option<&str>, name: &str) -> PathBuf {
    match local {
        None => PathBuf::from(name),
        Some(l) if Path::new(l).is_dir() || l.ends_with('/') || l.ends_with(std::path::MAIN_SEPARATOR) => Path::new(l).join(name),
        Some(l) => PathBuf::from(l),
    }
}

/// A `Content-Type` for an upload, from its name (the server files it by type).
pub fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "zip" => "application/zip",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// `put <local> [remote]`.
pub fn put(ctx: &Ctx, local: &str, remote_path: Option<&str>, force: bool) -> Result<()> {
    if ctx.is_webdav() {
        return super::webdav::put(ctx, local, remote_path, force);
    }
    let file = Path::new(local);
    let meta = std::fs::metadata(file).map_err(|e| CliError::not_found(format!("{local}: {e}")))?;
    if !meta.is_file() {
        return Err(CliError::usage(format!("{local} is not a file (upload folders file by file, or let the desktop client sync them)")));
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| CliError::usage("no file name"))?;
    let mut remote = ctx.remote()?;
    let wanted = remote_path.unwrap_or("");
    let mut path = clean(wanted)?;
    let mut existing = remote.stat(&path)?;
    if wanted.is_empty() || wanted.ends_with('/') || existing.as_ref().is_some_and(|e| e.is_folder) {
        path = join(&path, &name);
        existing = remote.stat(&path)?;
    }
    let saved = upload_to(&mut remote, file, &path, existing, &Overwrite::from_force(force))?;
    let verb = if saved.replaced { "Replaced" } else { "Uploaded" };
    done(ctx, json!({"path": path, "size": meta.len(), "replaced": saved.replaced, "etag": saved.etag}), &format!("{verb} {path}"))
}

/// What `put` and `save` do when a file is already at the cloud path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overwrite {
    /// Refuse (the default): a new file only, written with `If-None-Match: *`.
    Never,
    /// Replace whatever version is there (`--force`).
    Any,
    /// Replace only this version (`save --if-match`), written with `If-Match`.
    IfMatch(String),
}

impl Overwrite {
    pub fn from_force(force: bool) -> Self {
        if force { Self::Any } else { Self::Never }
    }
}

/// How a write goes, from what is at `path` now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Nothing is there: create it (`If-None-Match: *`).
    Create,
    /// Replace the version with this tag (`If-Match`), once the old bytes are kept; `None` when
    /// the listing gave none.
    Replace(Option<String>),
}

/// Decides a write from what is at `path` (`existing`: whether it is a folder, and its version
/// tag) and what `overwrite` allows. With `--if-match` a file that isn't there, or isn't that
/// version any more, is a conflict before anything is written or kept.
pub fn plan_write(path: &str, existing: Option<(bool, Option<&str>)>, overwrite: &Overwrite) -> Result<Plan> {
    match (existing, overwrite) {
        (Some((true, _)), _) => Err(CliError::conflict(format!("{path} is a folder"))),
        (None, Overwrite::IfMatch(_)) => {
            Err(CliError::conflict(format!("{path} isn't in the cloud any more (moved or deleted elsewhere); nothing was written")))
        }
        (None, _) => Ok(Plan::Create),
        (Some(_), Overwrite::Never) => Err(CliError::conflict(format!("{path} is already there (--force replaces it; the old version is kept)"))),
        (Some((_, etag)), Overwrite::Any) => Ok(Plan::Replace(etag.map(str::to_owned))),
        (Some((_, etag)), Overwrite::IfMatch(wanted)) => match etag {
            Some(now) if crate::cli::output::same_etag(now, wanted) => Ok(Plan::Replace(Some(now.to_owned()))),
            Some(now) => Err(CliError::conflict(format!("{path} changed in the cloud (it is version {now} now, not {wanted}); nothing was written"))),
            None => Err(CliError::conflict(format!("{path} is listed without a version tag, so it can't be shown unchanged; nothing was written"))),
        },
    }
}

/// What an upload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// It replaced a file (whose old version was kept).
    pub replaced: bool,
    /// The new version's tag, when the server said it.
    pub etag: Option<String>,
}

/// Uploads `file` to the cloud file `path` (`existing` is what is there now): refuses to replace
/// a file unless `overwrite` allows it, and snapshots the old version before it does.
pub(super) fn upload_to(remote: &mut Remote, file: &Path, path: &str, existing: Option<dav::Entry>, overwrite: &Overwrite) -> Result<Saved> {
    let path = path.to_owned();
    let name = split(&path).1.to_owned();
    let write = match plan_write(&path, existing.as_ref().map(|e| (e.is_folder, e.etag.as_deref())), overwrite)? {
        Plan::Create => Write::Create,
        Plan::Replace(etag) => {
            remote.session.snapshot(&path).map_err(CliError::transport)?;
            Write::Replace { etag: etag.unwrap_or_default() }
        }
    };
    let req = dav::upload(remote.server(), &path, Vec::new(), &write).header("Content-Type", content_type(&name));
    let resp = remote.session.upload(&req, file).map_err(CliError::transport)?;
    if resp.status == 409 {
        return Err(CliError::not_found(format!("the folder {} doesn't exist (make it with `craftcloud mkdir -p`)", split(&path).0)));
    }
    match dav::parse_upload(&resp).map_err(|e| CliError::api(&e).context(&path))? {
        Uploaded::Conflict => Err(CliError::conflict(format!("{path} changed on the server meanwhile; nothing was written"))),
        Uploaded::Saved { etag } if matches!(write, Write::Replace { .. }) => Ok(Saved { replaced: true, etag }),
        Uploaded::Saved { etag } => {
            // A new file starts with no history: versions still kept at this path belong to a
            // file that was here before (deleted, or moved without them), and restoring one of
            // them would put that file's bytes in place of this one (T-050). They go for good.
            remote.session.drop_versions(&path).map_err(CliError::transport)?;
            Ok(Saved { replaced: false, etag })
        }
    }
}

/// `mkdir [-p] <path>`.
pub fn mkdir(ctx: &Ctx, path: &str, parents: bool) -> Result<()> {
    if ctx.is_webdav() {
        return super::webdav::mkdir(ctx, path, parents);
    }
    let path = clean(path)?;
    if path.is_empty() {
        return Err(CliError::usage("which folder?"));
    }
    let mut remote = ctx.remote()?;
    let created = make_folder(&mut remote, &path, parents)?;
    let said = if created { format!("Made {path}") } else { format!("{path} was already there") };
    done(ctx, json!({"path": path, "created": created}), &said)
}

/// Makes a cloud folder (with `parents`: its missing parents too, and an existing one is fine).
/// Whether it made one.
pub(super) fn make_folder(remote: &mut Remote, path: &str, parents: bool) -> Result<bool> {
    let parts: Vec<&str> = path.split('/').collect();
    let steps: Vec<String> = if parents { (1..=parts.len()).map(|n| parts[..n].join("/")).collect() } else { vec![path.to_owned()] };
    let mut created = false;
    for step in steps {
        let resp = remote.call(&dav::create_folder(remote.server(), &step))?;
        match resp.status {
            201 => created = true,
            405 if parents => {}
            405 => return Err(CliError::conflict(format!("{step} is already there"))),
            409 => return Err(CliError::not_found(format!("the folder {} doesn't exist (-p makes it)", split(&step).0))),
            _ => expect_ok(&resp).map_err(|e| CliError::api(&e).context(&step))?,
        }
    }
    Ok(created)
}

/// `mv` and `cp`: into a folder when the destination is one, else to that name.
pub fn move_or_copy(ctx: &Ctx, from: &str, to: &str, copy: bool) -> Result<()> {
    if ctx.is_webdav() {
        // The storage trait has no copy (the sync never copies); `cp` is CraftCloud's server's.
        return if copy { Err(CliError::unsupported("copying on the server")) } else { super::webdav::mv(ctx, from, to) };
    }
    let (from, to_clean) = (clean(from)?, clean(to)?);
    if from.is_empty() {
        return Err(CliError::usage("the top of the cloud can't be moved or copied"));
    }
    let mut remote = ctx.remote()?;
    let source = remote.stat(&from)?.ok_or_else(|| CliError::not_found(format!("{from}: no such file or folder")))?;
    let into = to.ends_with('/') || to_clean.is_empty() || remote.stat(&to_clean)?.is_some_and(|e| e.is_folder);
    let dest = if into { join(&to_clean, split(&from).1) } else { to_clean };
    if dest == from || dest.starts_with(&format!("{from}/")) {
        return Err(CliError::usage(format!("{from} can't go into itself")));
    }
    let req = if copy {
        dav::copy_to(remote.server(), &from, &dest, source.is_folder)
    } else {
        dav::move_to(remote.server(), &from, &dest, source.is_folder)
    };
    let resp = remote.call(&req)?;
    match resp.status {
        412 => return Err(CliError::conflict(format!("{dest} is already there"))),
        409 => return Err(CliError::not_found(format!("the folder {} doesn't exist", split(&dest).0))),
        _ => expect_ok(&resp).map_err(|e| CliError::api(&e))?,
    }
    if !copy {
        // The kept versions go along (T-050); a copy starts with none.
        remote.session.move_versions(&from, &dest).map_err(CliError::transport)?;
    }
    let verb = if copy { "Copied" } else { "Moved" };
    done(ctx, json!({"from": from, "to": dest}), &format!("{verb} {from} to {dest}"))
}

/// `rm <path>…`: to the trash.
pub fn rm(ctx: &Ctx, paths: &[String]) -> Result<()> {
    if ctx.is_webdav() {
        return super::webdav::rm(ctx, paths);
    }
    let mut remote = ctx.remote()?;
    let mut trashed = Vec::new();
    for p in paths {
        let path = clean(p)?;
        if path.is_empty() {
            return Err(CliError::usage("the top of the cloud can't go to the trash"));
        }
        let found = remote.resolve(&path)?;
        let req = match found.kind() {
            Kind::File => rest_files::trash(remote.server(), found.id()),
            Kind::Folder => craftcloud_core::api::folders::trash(remote.server(), found.id()),
        };
        remote.ask(&req, expect_ok).map_err(|e| e.context(&path))?;
        if !ctx.json {
            println!("Moved {path} to the trash");
        }
        trashed.push(path);
    }
    if ctx.json { print_json(&json!({"trashed": trashed})) } else { Ok(()) }
}

fn trash_items(remote: &mut Remote) -> Result<Vec<TrashItem>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = remote.ask(&trash::list(remote.server(), Some(200), cursor.as_deref()), trash::parse_list_all)?;
        out.extend(page.items);
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => return Ok(out),
        }
    }
}

/// Finds one trashed item by id, old path or (when only one has it) name.
pub fn pick<'a>(items: &'a [TrashItem], wanted: &str) -> Result<&'a TrashItem> {
    let path = wanted.trim_matches('/');
    if let Some(i) = items.iter().find(|i| i.resource.id() == wanted) {
        return Ok(i);
    }
    let by_path: Vec<&TrashItem> = items.iter().filter(|i| i.resource.cloud_path().as_deref() == Some(path)).collect();
    let matches = if by_path.is_empty() { items.iter().filter(|i| i.resource.name() == path).collect() } else { by_path };
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(CliError::not_found(format!("{wanted} isn't in the trash"))),
        more => Err(CliError::conflict(format!(
            "{} items in the trash match {wanted}; use the id from `craftcloud trash ls --json` ({})",
            more.len(),
            more.iter().map(|i| i.resource.id()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

fn trash_item(i: &TrashItem) -> Item {
    // A trashed file can't be written to: no version tag.
    Item { trashed_at: Some(i.trashed_at.clone()), expires_at: Some(i.expires_at.clone()), etag: None, ..Item::listed(&i.resource) }
}

/// `trash ls|restore|rm|empty`.
pub fn trash(ctx: &Ctx, cmd: TrashCmd) -> Result<()> {
    if ctx.is_webdav() {
        // CraftCloud's own trash on the server (`.craftcloud/trash/`), through the storage trait.
        return super::webdav::trash(ctx, cmd);
    }
    let mut remote = ctx.remote()?;
    match cmd {
        TrashCmd::Ls { all } => {
            let items: Vec<Item> = trash_items(&mut remote)?.iter().filter(|i| all || !i.resource.is_internal()).map(trash_item).collect();
            print_items(ctx, &items, |i| i.path.clone())
        }
        TrashCmd::Restore { item, to, rename } => {
            let all = trash_items(&mut remote)?;
            let found = pick(&all, &item)?;
            let path = found.resource.cloud_path().unwrap_or_default();
            let dest = restore(&mut remote, found, &path, to.as_deref(), rename)?;
            let human = if dest == path { format!("Restored {path}") } else { format!("Restored {path} as {dest}") };
            done(ctx, json!({"restored": dest, "from": path, "id": found.resource.id()}), &human)
        }
        TrashCmd::Rm { item } => {
            let all = trash_items(&mut remote)?;
            let found = pick(&all, &item)?;
            let path = found.resource.cloud_path().unwrap_or_default();
            remote.ask(&trash::delete(remote.server(), found.resource.id()), expect_ok)?;
            drop_histories(&mut remote, std::slice::from_ref(&path))?;
            done(ctx, json!({"deleted": path, "id": found.resource.id()}), &format!("Deleted {path} for good"))
        }
        TrashCmd::Empty { yes } => {
            if !yes {
                confirm("Delete everything in the trash for good?")?;
            }
            let paths: Vec<String> =
                trash_items(&mut remote)?.iter().filter(|i| !i.resource.is_internal()).filter_map(|i| i.resource.cloud_path()).collect();
            remote.ask(&trash::empty(remote.server()), expect_ok)?;
            drop_histories(&mut remote, &paths)?;
            done(ctx, json!({"emptied": true}), "The trash is empty.")
        }
    }
}

/// The kept versions of files deleted for good go too, so they don't take room for nothing
/// (T-049); a path where a file is again keeps the history, which is that file's.
fn drop_histories(remote: &mut Remote, paths: &[String]) -> Result<()> {
    for path in paths {
        if !path.is_empty() && remote.stat(path)?.is_none() {
            remote.session.drop_versions(path).map_err(CliError::transport)?;
        }
    }
    Ok(())
}

/// Puts a trashed item back: where it was, or at `to` (`--as`), or, with `rename`, as
/// "name (restored).ext" when its place is taken. Returns where it is now. OxiCloud restores only
/// to the old place and fails (500) when that is taken (T-047), so the item there steps aside
/// for a moment (a WebDAV move, which keeps its id) while the restored one moves on.
fn restore(remote: &mut Remote, found: &TrashItem, path: &str, to: Option<&str>, rename: bool) -> Result<String> {
    let is_folder = found.resource.kind() == Kind::Folder;
    let taken = remote.stat(path)?.is_some();
    let dest = match to {
        Some(to) => clean(to)?,
        None if rename && taken => {
            let mut n = 1;
            loop {
                let candidate = craftcloud_core::names::restored_name(path, is_folder, n);
                if remote.stat(&candidate)?.is_none() {
                    break candidate;
                }
                n += 1;
            }
        }
        None => path.to_owned(),
    };
    if dest == path && taken {
        return Err(CliError::conflict(format!(
            "{path} is already there. Restore it under another name: `craftcloud trash restore {} --rename` (as \"{}\") or `--as PATH`.",
            found.resource.id(),
            split(&craftcloud_core::names::restored_name(path, is_folder, 1)).1
        )));
    }
    if dest.is_empty() {
        return Err(CliError::usage("restore it into a folder, not as the top of the cloud"));
    }
    if dest != path && remote.stat(&dest)?.is_some() {
        return Err(CliError::conflict(format!("{dest} is already there")));
    }
    let restore_it = |remote: &mut Remote| remote.ask(&trash::restore(remote.server(), found.resource.id()), expect_ok);
    let mv = |remote: &mut Remote, from: &str, to: &str| -> Result<()> {
        let resp = remote.call(&dav::move_to(remote.server(), from, to, is_folder))?;
        match resp.status {
            412 => Err(CliError::conflict(format!("{to} is already there"))),
            409 => Err(CliError::not_found(format!("the folder {} doesn't exist", split(to).0))),
            _ => expect_ok(&resp).map_err(|e| CliError::api(&e)),
        }
    };
    if dest == path {
        restore_it(remote)?;
        return Ok(dest);
    }
    // The destination's folder must be there before anything moves.
    if remote.stat(split(&dest).0)?.is_none_or(|e| !e.is_folder) {
        return Err(CliError::not_found(format!("the folder {} doesn't exist", split(&dest).0)));
    }
    let aside = taken.then(|| join(split(path).0, &format!(".{}.restoring", split(path).1)));
    if let Some(aside) = &aside {
        mv(remote, path, aside)?;
    }
    let result = restore_it(remote).and_then(|()| mv(remote, path, &dest));
    if let Some(aside) = &aside {
        // Back where it was, whatever happened.
        mv(remote, aside, path).map_err(|e| e.context(&format!("{path} is at {aside} now; move it back")))?;
    }
    result.map(|()| dest)
}

/// Asks yes/no on the terminal; without one, refuses (pass --yes).
pub fn confirm(question: &str) -> Result<()> {
    if !super::stdin_is_terminal() {
        return Err(CliError::usage(format!("{question} Pass --yes to confirm.")));
    }
    eprint!("{question} [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).map_err(|e| CliError::error(e.to_string()))?;
    if matches!(answer.trim(), "y" | "Y" | "yes") { Ok(()) } else { Err(CliError::new(Fail::Error, "Nothing was deleted.")) }
}

/// `star` / `unstar`.
pub fn star(ctx: &Ctx, path: &str, on: bool) -> Result<()> {
    let path = clean(path)?;
    if ctx.is_webdav() {
        return Err(CliError::unsupported("stars"));
    }
    let mut remote = ctx.remote()?;
    let found = remote.resolve(&path)?;
    let req =
        if on { favorites::add(remote.server(), found.kind(), found.id()) } else { favorites::remove(remote.server(), found.kind(), found.id()) };
    let resp = remote.call(&req)?;
    // Starring a starred item (or unstarring one that isn't) changes nothing; that's fine.
    if !(resp.is_success() || (!on && resp.status == 404) || (on && resp.status == 409)) {
        expect_ok(&resp).map_err(|e| CliError::api(&e).context(&path))?;
    }
    done(ctx, json!({"path": path, "starred": on}), &format!("{} {path}", if on { "Starred" } else { "Unstarred" }))
}

/// `starred [--app]`.
pub fn starred(ctx: &Ctx, all: bool, app: Option<&str>) -> Result<()> {
    let app = craft::pick_app(app)?;
    if ctx.is_webdav() {
        return Err(CliError::unsupported("stars"));
    }
    let mut remote = ctx.remote()?;
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = remote.ask(&favorites::list(remote.server(), Some(200), cursor.as_deref()), favorites::parse_list_all)?;
        items.extend(page.items.iter().filter(|f| all || !f.resource.is_internal()).map(|f| Item {
            starred_at: Some(f.favorited_at.clone()),
            starred: true,
            ..Item::listed(&f.resource)
        }));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    print_items(ctx, &craft::for_app(ctx, items, app), |i| i.path.clone())
}

/// `recent [--clear] [--app]`.
pub fn recent(ctx: &Ctx, clear: bool, all: bool, app: Option<&str>) -> Result<()> {
    let app = craft::pick_app(app)?;
    if ctx.is_webdav() {
        return Err(CliError::unsupported("the recent list"));
    }
    let mut remote = ctx.remote()?;
    if clear {
        remote.ask(&favorites::clear_recent(remote.server()), expect_ok)?;
        return done(ctx, json!({"cleared": true}), "Cleared the recent list.");
    }
    let page = remote.ask(&favorites::recent(remote.server(), Some(100), None), favorites::parse_recent_all)?;
    let items: Vec<Item> = page
        .items
        .iter()
        .filter(|r| all || !r.resource.is_internal())
        .map(|r| Item { opened_at: Some(r.accessed_at.clone()), ..Item::listed(&r.resource) })
        .collect();
    print_items(ctx, &craft::for_app(ctx, items, app), |i| i.path.clone())
}

/// What to search for (`search`'s options).
pub struct SearchArgs<'a> {
    pub query: &'a str,
    pub kind: Option<&'a str>,
    pub within: Option<&'a str>,
    pub limit: u32,
    pub all: bool,
    pub app: Option<&'a str>,
}

/// `search <query> [--type] [--in] [--limit] [--app]`. With `--app` the server is asked for its
/// most (200) and the files the app opens are kept, up to `--limit`.
pub fn search(ctx: &Ctx, args: &SearchArgs) -> Result<()> {
    let SearchArgs { query, kind, within, limit, all, app } = *args;
    let app = craft::pick_app(app)?;
    if ctx.is_webdav() {
        return Err(CliError::unsupported("search"));
    }
    let kind = match kind {
        None => None,
        Some(k) => Some(TypeFilter::parse(k).ok_or_else(|| {
            CliError::usage(format!("--type {k}: use one of {}", TypeFilter::ALL.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(", ")))
        })?),
    };
    let mut remote = ctx.remote()?;
    let folder_id = match within {
        Some(p) => match remote.resolve(&clean(p)?)? {
            Resource::Folder(f) => Some(f.id),
            Resource::File(_) => return Err(CliError::usage(format!("--in {p}: that's a file"))),
        },
        None => None,
    };
    let asked = if app.is_some() { 200 } else { limit.clamp(1, 200) };
    let q = search::Query { kind, folder_id, limit: Some(asked), ..search::Query::new(query) };
    let found = remote.ask(&search::search(remote.server(), &q), search::parse_search_all)?;
    let items: Vec<Item> =
        found.items.iter().filter(|h| all || !h.resource.is_internal()).map(|h| Item { score: Some(h.score), ..Item::listed(&h.resource) }).collect();
    let mut items = craft::for_app(ctx, items, app);
    items.truncate(limit.clamp(1, 200) as usize);
    print_items(ctx, &items, |i| i.path.clone())
}

#[cfg(test)]
mod tests {
    use super::{Overwrite, Plan, content_type, pick, plan_write, target};
    use craftcloud_core::api::resources::{File, Resource};
    use craftcloud_core::api::trash::TrashItem;
    use std::path::PathBuf;

    fn trashed(id: &str, path: &str) -> TrashItem {
        let name = path.rsplit('/').next().unwrap().to_owned();
        let file = File {
            id: id.into(),
            name,
            path: format!("Personal/{path}"),
            size: 1,
            mime_type: String::new(),
            folder_id: None,
            created_at: String::new(),
            modified_at: String::new(),
            category: String::new(),
            content_hash: String::new(),
            etag: String::new(),
            is_favorite: false,
            is_shared: false,
        };
        TrashItem { resource: Resource::File(file), trashed_at: String::new(), expires_at: String::new() }
    }

    #[test]
    fn trash_items_are_found_by_id_path_or_name() {
        let items = [trashed("1", "Documents/a.txt"), trashed("2", "Old/a.txt"), trashed("3", "b.txt")];
        assert_eq!(pick(&items, "2").unwrap().resource.id(), "2");
        assert_eq!(pick(&items, "/Documents/a.txt").unwrap().resource.id(), "1");
        assert_eq!(pick(&items, "b.txt").unwrap().resource.id(), "3");
        assert_eq!(pick(&items, "a.txt").unwrap_err().kind.code(), 5, "two match: ambiguous");
        assert_eq!(pick(&items, "c.txt").unwrap_err().kind.code(), 4);
    }

    #[test]
    fn downloads_land_where_asked() {
        assert_eq!(target(None, "a.txt"), PathBuf::from("a.txt"));
        assert_eq!(target(Some("out/"), "a.txt"), PathBuf::from("out/").join("a.txt"));
        assert_eq!(target(Some("b.txt"), "a.txt"), PathBuf::from("b.txt"));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(target(Some(dir.path().to_str().unwrap()), "a.txt"), dir.path().join("a.txt"));
    }

    #[test]
    fn uploads_say_what_they_are() {
        assert_eq!(content_type("notes.MD"), "text/markdown");
        assert_eq!(content_type("report.pdf"), "application/pdf");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }

    /// What a write does with what is there: never replaces by accident, and `--if-match` only
    /// replaces the version named, deciding before any old version is kept.
    #[test]
    fn writes_replace_only_what_they_may() {
        let file = |tag: &'static str| Some((false, Some(tag)));
        let code = |r: crate::cli::output::Result<Plan>| r.unwrap_err().kind.code();
        let if_match = |t: &str| Overwrite::IfMatch(t.into());

        // Nothing there: created (If-None-Match: *), unless the caller expected a version.
        assert_eq!(plan_write("a.md", None, &Overwrite::Never).unwrap(), Plan::Create);
        assert_eq!(plan_write("a.md", None, &Overwrite::Any).unwrap(), Plan::Create);
        let gone = plan_write("a.md", None, &if_match("\"v1\"")).unwrap_err();
        assert_eq!((gone.kind.code(), gone.kind.name()), (5, "conflict"), "{}", gone.message);

        // A file there: refused by default, replaced with --force under the listed tag.
        assert_eq!(code(plan_write("a.md", file("\"v1\""), &Overwrite::Never)), 5);
        assert_eq!(plan_write("a.md", file("\"v1\""), &Overwrite::Any).unwrap(), Plan::Replace(Some("\"v1\"".into())));

        // --if-match: the same version (quotes or not) is replaced, under the server's own tag.
        assert_eq!(plan_write("a.md", file("\"v1\""), &if_match("\"v1\"")).unwrap(), Plan::Replace(Some("\"v1\"".into())));
        assert_eq!(plan_write("a.md", file("\"v1\""), &if_match("v1")).unwrap(), Plan::Replace(Some("\"v1\"".into())));
        // Another version, or one without a tag: a conflict, nothing written or kept.
        let changed = plan_write("a.md", file("\"v2\""), &if_match("\"v1\"")).unwrap_err();
        assert_eq!((changed.kind.code(), changed.kind.name()), (5, "conflict"));
        assert!(changed.message.contains("changed in the cloud") && changed.message.contains("nothing was written"), "{}", changed.message);
        assert_eq!(code(plan_write("a.md", Some((false, None)), &if_match("\"v1\""))), 5);

        // A folder is never written over.
        for o in [Overwrite::Never, Overwrite::Any, if_match("\"v1\"")] {
            assert_eq!(code(plan_write("a", Some((true, Some("\"d\""))), &o)), 5);
        }
    }
}
