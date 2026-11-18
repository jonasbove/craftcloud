//! CraftCloud's own file versions on a server that keeps none itself: the bytes an overwrite is
//! about to replace are copied to `.craftcloud/versions/<path>/<time>` under the WebDAV base, and
//! the newest [`versions::VERSIONS_KEPT`] are kept (`craftcloud_core::api::versions` has the
//! steps). One body of code for every driver that keeps versions this way: OxiCloud's
//! ([`crate::session::Session::snapshot`]) and the generic WebDAV driver's, each under its own
//! base, over the small [`Transport`] the probe runner defined. Listings are read against the
//! base ([`parse_listing`]): an href outside it fails the listing.

use craftcloud_core::api::{dav, versions};
use craftcloud_core::storage::probe::{Base, Listed, parse_listing};

use super::probe::Transport;

/// Keeps the cloud version of `path` in `.craftcloud/versions/<path>/<now>` under `base`, then
/// deletes all but the newest [`versions::VERSIONS_KEPT`]. Nothing to keep (no such file) is
/// fine, and so is a newest version that already holds these bytes.
pub fn keep(transport: &mut dyn Transport, base: &Base, path: &str) -> Result<(), String> {
    // Already kept (an overwrite tried again after it failed, a save with the same bytes):
    // nothing to add.
    let current = transport.call(&dav::stat_under(base.url(), path))?;
    if current.status == 404 {
        return Ok(());
    }
    let entry: Option<Listed> = parse_listing(&current, base).ok().and_then(|e| e.into_iter().next());
    // When the content was last changed: the copy gets the time of the copy (T-019).
    let modified = entry
        .as_ref()
        .and_then(|e| e.modified.as_deref())
        .and_then(|m| jiff::fmt::rfc2822::parse(m).ok())
        .map(|z| z.timestamp().strftime(versions::MODIFIED_FORMAT).to_string());
    if let Some(etag) = entry.and_then(|e| e.etag)
        && versions::already_kept(&etag, &list(transport, base, path)?)
    {
        return Ok(());
    }
    for folder in versions::make_folders_under(base.url(), path) {
        let resp = transport.call(&folder)?;
        if !dav::folder_exists_after(&resp) {
            return Err(format!("can't make the versions folder for {path}: the server answered {}", resp.status));
        }
    }
    let stamp = versions::name_for(&jiff::Timestamp::now().strftime(versions::STAMP_FORMAT).to_string(), modified.as_deref());
    let resp = transport.call(&versions::snapshot_under(base.url(), path, &stamp))?;
    versions::snapshot_done(&resp).map_err(|e| format!("keeping the previous version of {path} failed: {e}"))?;
    let names: Vec<String> = list(transport, base, path)?.into_iter().map(|v| v.stamp).collect();
    for old in versions::to_prune(names) {
        let _ = transport.call(&versions::delete_under(base.url(), path, &old));
    }
    Ok(())
}

/// The kept versions of `path` under `base`, newest first. A file that never had one has no
/// versions folder (404): an empty list, not an error.
pub fn list(transport: &mut dyn Transport, base: &Base, path: &str) -> Result<Vec<versions::Version>, String> {
    let resp = transport.call(&versions::list_under(base.url(), path))?;
    if resp.status == 404 {
        return Ok(Vec::new());
    }
    let mut out: Vec<versions::Version> = parse_listing(&resp, base)?
        .into_iter()
        .filter(|e| !e.is_folder)
        .map(|e| versions::Version { stamp: e.name().to_owned(), size: e.size, etag: e.etag })
        .collect();
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    Ok(out)
}

/// Puts version `stamp` of `path` back under `base`, after keeping the current file as a version
/// of its own (so the restore can be undone), exactly as OxiCloud's driver does. The version must
/// be one of the kept ones.
pub fn restore(transport: &mut dyn Transport, base: &Base, path: &str, stamp: &str) -> Result<(), String> {
    if !list(transport, base, path)?.iter().any(|v| v.stamp == stamp) {
        return Err(format!("{path} has no version {stamp}"));
    }
    keep(transport, base, path)?;
    let resp = transport.call(&versions::restore_under(base.url(), path, stamp))?;
    craftcloud_core::api::expect_ok(&resp).map_err(|e| format!("restoring {path} failed: {e}"))
}

/// Takes the versions of `from` (a file, or a folder's files) along to `to` after a rename or
/// move. No history is fine; where `to` already has one, the versions are added to it.
pub fn follow_move(transport: &mut dyn Transport, base: &Base, from: &str, to: &str) -> Result<(), String> {
    for folder in versions::history_parents(to) {
        let resp = transport.call(&dav::create_folder_under(base.url(), &folder))?;
        if !dav::folder_exists_after(&resp) {
            return Err(format!("can't make the versions folder for {to}: the server answered {}", resp.status));
        }
    }
    let resp = transport.call(&versions::move_history_under(base.url(), from, to))?;
    if resp.is_success() || resp.status == 404 {
        return Ok(());
    }
    if resp.status != 412 {
        return Err(format!("moving the versions of {from}: the server answered {}", resp.status));
    }
    // `to` has a history already (an earlier file of that name): add these to it.
    let (old, new) = (versions::versions_folder(from), versions::versions_folder(to));
    let listed = transport.call(&dav::list_under(base.url(), &old))?;
    let entries = parse_listing(&listed, base)?;
    for e in entries.iter().filter(|e| e.path != old) {
        let _ = transport.call(&dav::move_to_under(base.url(), &e.path, &format!("{new}/{}", e.name()), e.is_folder));
    }
    let left = transport.call(&dav::list_under(base.url(), &old)).ok().and_then(|r| parse_listing(&r, base).ok()).map_or(0, |e| e.len());
    if left <= 1 {
        let _ = transport.call(&dav::delete_under(base.url(), &old, true));
    }
    Ok(())
}
