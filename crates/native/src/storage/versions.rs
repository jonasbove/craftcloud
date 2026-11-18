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
    // `to` has a history already (an earlier file of that name): add these to it, one by one.
    let (old, new) = (versions::versions_folder(from), versions::versions_folder(to));
    let listed = transport.call(&dav::list_under(base.url(), &old))?;
    let entries = parse_listing(&listed, base)?;
    let mut all_moved = true;
    for e in entries.iter().filter(|e| e.path != old) {
        let moved = transport.call(&dav::move_to_under(base.url(), &e.path, &format!("{new}/{}", e.name()), e.is_folder));
        if !moved.is_ok_and(|r| r.is_success()) {
            all_moved = false;
        }
    }
    // The old folder goes (for good, inside `.craftcloud/versions/`) only when every version
    // moved and a fresh listing shows nothing left in it but the folder itself. A listing that
    // fails is unknown, never empty: the folder stays, with whatever it still holds.
    let empty_now = all_moved
        && transport
            .call(&dav::list_under(base.url(), &old))
            .ok()
            .and_then(|r| parse_listing(&r, base).ok())
            .is_some_and(|e| e.iter().all(|x| x.path == old));
    if empty_now {
        let _ = transport.call(&dav::delete_under(base.url(), &old, true));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use craftcloud_core::storage::probe::fake::{FakeDav, Flags};

    use super::follow_move;
    use crate::fake_server::{Answer, FakeServer, Seen};
    use crate::storage::webdav::tests::{base, session};

    const BASE_PATH: &str = "/remote.php/dav/files/john";
    const OLD: &str = "/remote.php/dav/files/john/.craftcloud/versions/Docs/a.txt/";

    /// A scripted server with histories for `Docs/a.txt` (two versions) and `Docs/b.txt` (one), so
    /// moving a.txt's history onto b.txt's goes version by version; `fail_second_listing` makes
    /// the second listing of a.txt's folder (the one after the moves) answer 500.
    fn histories(fail_second_listing: bool) -> (FakeServer, Arc<Mutex<FakeDav>>) {
        let dav = Arc::new(Mutex::new(FakeDav::new(BASE_PATH, Flags::default())));
        {
            let mut dav = dav.lock().unwrap();
            dav.add_folder(".craftcloud/versions/Docs/a.txt", None);
            dav.add_folder(".craftcloud/versions/Docs/b.txt", None);
            dav.add_file(".craftcloud/versions/Docs/a.txt/20261118T100000.000Z", b"a1");
            dav.add_file(".craftcloud/versions/Docs/a.txt/20261118T110000.000Z", b"a2");
            dav.add_file(".craftcloud/versions/Docs/b.txt/20261118T090000.000Z", b"b1");
        }
        let shared = Arc::clone(&dav);
        let listings = Arc::new(Mutex::new(0usize));
        let server = FakeServer::start_full(move |seen: &Seen| {
            if seen.method == "PROPFIND" && seen.path == OLD {
                let mut n = listings.lock().unwrap_or_else(PoisonError::into_inner);
                *n += 1;
                if fail_second_listing && *n == 2 {
                    return Answer::new(500, "");
                }
            }
            let mut dav = shared.lock().unwrap_or_else(PoisonError::into_inner);
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        (server, dav)
    }

    /// A history moved onto an existing one goes version by version; the old folder is deleted
    /// only when a fresh listing shows it empty. When that listing fails, the folder stays: a
    /// failed listing is unknown, never empty (the sync never deletes on uncertainty).
    #[test]
    fn an_old_history_folder_stays_when_its_listing_fails() {
        for (fails, deleted) in [(false, true), (true, false)] {
            let (server, dav) = histories(fails);
            let mut s = session(&server);
            follow_move(&mut s, &base(&server), "Docs/a.txt", "Docs/b.txt").unwrap();
            let deletes: Vec<String> = server.seen().iter().filter(|r| r.method == "DELETE").map(|r| r.path.clone()).collect();
            let dav = dav.lock().unwrap();
            assert_eq!(dav.body(".craftcloud/versions/Docs/b.txt/20261118T100000.000Z"), Some(b"a1".as_slice()), "fails={fails}: moved");
            assert_eq!(dav.body(".craftcloud/versions/Docs/b.txt/20261118T110000.000Z"), Some(b"a2".as_slice()), "fails={fails}: moved");
            assert_eq!(dav.body(".craftcloud/versions/Docs/b.txt/20261118T090000.000Z"), Some(b"b1".as_slice()), "fails={fails}: kept");
            assert_eq!(dav.has(".craftcloud/versions/Docs/a.txt"), !deleted, "fails={fails}: {deletes:?}");
            assert_eq!(deletes.len(), usize::from(deleted), "fails={fails}: {deletes:?}");
            assert!(deletes.iter().all(|p| p == OLD), "only the old history folder itself, inside .craftcloud/versions/: {deletes:?}");
        }
    }
}
