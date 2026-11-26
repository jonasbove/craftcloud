//! Runs the check on connect (docs/WEBDAV-STORAGE.md § 7.5) over HTTP. The steps and the verdict
//! are `craftcloud_core::storage::probe`'s; this sends each request the probe asks for, in
//! `.craftcloud/probe/<device>-<random>/`, and keeps the result in `storage-check.json` in the
//! settings folder.
//!
//! What this file holds the line on:
//! - **No unguarded write leaves here.** The probe builds its requests with their guards; the
//!   runner checks every one again before sending ([`guarded`]): a `PUT` without `If-Match` or
//!   `If-None-Match: *`, a `MOVE` or `COPY` without `Overwrite: F`, or a `DELETE` anywhere but
//!   inside `.craftcloud/probe/` (an [`OwnPath`]) stops the probe, refused.
//! - **A probe that doesn't finish is refused.** A transport error, a request that takes longer
//!   than [`Limits::step`], or a whole run longer than [`Limits::total`] ends it, and the folder
//!   it left is swept by the next run, by the server's clock.
//! - **The transport is small** ([`Transport`]: `call` and `upload`, the two shapes `Session` has),
//!   so the generic WebDAV driver that follows can run the probe with an app password too.

use std::path::Path;
use std::time::{Duration, Instant};

use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
pub use craftcloud_core::storage::probe::{Base, PROBE_DIR, Probe, Profile, Request, StoredCheck, Verdict};

use super::OwnPath;
use crate::session::Session;

/// Sends requests for the probe: the two shapes [`Session`] has.
pub trait Transport {
    /// Sends a request and waits for the whole answer. `Err` is a transport error (no answer).
    fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse, String>;
    /// Like `call`, with the body streamed from a file.
    fn upload(&mut self, req: &HttpRequest, file: &Path) -> Result<HttpResponse, String>;
}

impl Transport for Session {
    fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
        Session::call(self, req)
    }

    fn upload(&mut self, req: &HttpRequest, file: &Path) -> Result<HttpResponse, String> {
        Session::upload(self, req, file)
    }
}

impl Transport for crate::session::AppPasswordSession {
    fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
        Self::call(self, req)
    }

    fn upload(&mut self, req: &HttpRequest, file: &Path) -> Result<HttpResponse, String> {
        Self::upload(self, req, file)
    }
}

/// How long a probe may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// One request, from sending to the whole answer (the HTTP layer's own connect and idle
    /// limits stop a dead connection sooner).
    pub step: Duration,
    /// The whole probe.
    pub total: Duration,
    /// How long to wait before a write that must land in a later second than the write before it
    /// (`Request::settles`): Nextcloud and ownCloud keep a file's version tag when its modified
    /// time in whole seconds is unchanged. Zero in tests against a scripted server.
    pub settle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { step: Duration::from_secs(30), total: Duration::from_secs(180), settle: Duration::from_millis(1100) }
    }
}

/// The name of this device's probe folder: `<device>-<random>`, letters, digits and dashes only.
pub fn folder_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let device: String = crate::device_name().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(32).collect();
    let device = device.trim_matches('-');
    let device = if device.is_empty() { "device" } else { device };
    // `RandomState` is seeded at random per process; the time and the pid vary the rest.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.write_u32(std::process::id());
    format!("{device}-{:016x}", hasher.finish())
}

/// Why a request the probe asked for may not be sent: the guard it must carry is missing, or it
/// would delete outside `.craftcloud/probe/`. The probe never builds such a request; this is the
/// second check, so that a mistake there can't reach a server. `probe_folder` is this probe's
/// folder (`.craftcloud/probe/<device>-<random>`): on another DAV root (a server's trash bin or
/// versions, which a known-server check reads) only a listing, a `GET` and the restore `MOVE` of
/// the probe's own file may go, never a `DELETE` (a person's trash is never purged, § 16, Q8) and
/// never a write.
pub fn guarded(req: &Request, probe_folder: &str) -> Result<(), String> {
    if let Some(root) = &req.root {
        return guarded_on_root(req, root, probe_folder);
    }
    match req.method {
        Method::Put if req.header_value("If-Match").is_none() && req.header_value("If-None-Match") != Some("*") => {
            Err(format!("refused to send an unguarded PUT to {}", req.path))
        }
        Method::Move | Method::Copy if req.header_value("Overwrite") != Some("F") => {
            Err(format!("refused to send a {} without Overwrite: F", req.method.as_str()))
        }
        Method::Move | Method::Copy if !req.destination.as_ref().is_some_and(|(to, _)| inside_probe_dir(to)) => {
            Err(format!("refused to send a {} outside {PROBE_DIR}/", req.method.as_str()))
        }
        Method::Delete if OwnPath::new(&req.path).is_none() || !inside_probe_dir(&req.path) => {
            Err(format!("refused to delete {} (outside {PROBE_DIR}/)", req.path))
        }
        _ => Ok(()),
    }
}

fn inside_probe_dir(path: &str) -> bool {
    path.strip_prefix(PROBE_DIR).is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
}

/// A known-server check's request on another DAV root: read freely; move only the probe's own
/// trashed file back, in one of two shapes (the server said it came from inside the probe's
/// folder either way): Nextcloud's, `trash/<name>` → `restore/<name>` on the trash bin itself;
/// or ownCloud's, an item right under the root → exactly the cloud path it came from, under the
/// files base, with `Overwrite: F` (there the destination is a name in the person's tree, and
/// the server would take any other). Nothing else.
fn guarded_on_root(req: &Request, root: &str, probe_folder: &str) -> Result<(), String> {
    match req.method {
        Method::Propfind | Method::Get => Ok(()),
        Method::Move => {
            let own = req.restores.as_deref().is_some_and(|from| {
                !probe_folder.is_empty() && from.strip_prefix(probe_folder).is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
            });
            let is_restore = if req.destination_under_base {
                // ownCloud's shape: the item itself (one segment), to where it was, never onto a name.
                let item = !req.path.is_empty() && !req.path.contains('/');
                let to_origin = req.destination.as_ref().is_some_and(|(to, _)| Some(to.as_str()) == req.restores.as_deref());
                item && to_origin && req.header_value("Overwrite") == Some("F")
            } else {
                // Nextcloud's shape. No `Overwrite` rule here: the restore address is the server's
                // endpoint, not a name in the person's tree (Nextcloud answers 412 to `Overwrite: F`
                // there); what guards the move is that it is a restore, of the probe's own file,
                // to where it was.
                let name = req.path.strip_prefix("trash/").filter(|n| !n.is_empty() && !n.contains('/'));
                let restored = req.destination.as_ref().and_then(|(to, _)| to.strip_prefix("restore/"));
                name.is_some() && restored == name
            };
            if !is_restore {
                Err(format!("refused to send a MOVE on {root} that isn't the restore of a trashed item to where it was"))
            } else if !own {
                Err(format!("refused to restore {} from {root}: not the probe's own file", req.path))
            } else {
                Ok(())
            }
        }
        Method::Delete => Err(format!("refused to delete {} on {root}: a server's trash and versions are the person's, never purged", req.path)),
        other => Err(format!("refused to send a {} on {root}", other.as_str())),
    }
}

/// Runs the probe to its end (or its first failure) and gives the verdict. `folder_name` is this
/// device's probe folder ([`folder_name`]).
pub fn run(transport: &mut dyn Transport, base: &Base, folder_name: &str, profile: Profile<'_>, limits: Limits) -> Verdict {
    let mut probe = Probe::new(base.clone(), folder_name, profile);
    let started = Instant::now();
    while let Some(req) = probe.next() {
        if let Err(why) = guarded(&req, probe.folder()) {
            probe.fail(&why);
            break;
        }
        if started.elapsed() > limits.total {
            probe.fail(&format!("the check took longer than {} s", limits.total.as_secs()));
            break;
        }
        if req.settles && !limits.settle.is_zero() {
            std::thread::sleep(limits.settle);
        }
        let sent = Instant::now();
        let answer = transport.call(&req.to_http(base));
        let took = sent.elapsed();
        match answer {
            Ok(resp) if took > limits.step => {
                probe.fail(&format!("one request took {} s, more than {} s", took.as_secs(), limits.step.as_secs()));
                // The answer came, but too late to trust the server with a sync: nothing more is sent.
                let _ = resp;
                break;
            }
            Ok(resp) => probe.observe(&resp),
            Err(why) => {
                probe.fail(&why);
                break;
            }
        }
    }
    let verdict = probe.verdict();
    log::info!("server check of {}: {} ({} steps)", base.url(), verdict.mode.as_str(), verdict.steps.len());
    verdict
}

/// The saved check's file name in the settings folder.
pub const CHECK_FILE: &str = "storage-check.json";

/// Saves a check in `dir` (the settings folder), whole or not at all.
pub fn save_check(dir: &Path, check: &StoredCheck) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    let path = dir.join(CHECK_FILE);
    craft_atomic::atomic_write(&path, check.to_json().as_bytes()).map_err(|e| format!("can't save {}: {e}", path.display()))
}

/// The check saved in `dir`, if there is a readable one.
pub fn load_check(dir: &Path) -> Option<StoredCheck> {
    std::fs::read(dir.join(CHECK_FILE)).ok().and_then(|bytes| StoredCheck::from_json(&bytes))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use craftcloud_core::api::auth::Tokens;
    use craftcloud_core::http::{HttpRequest, HttpResponse, Method};
    use craftcloud_core::storage::probe::fake::{FakeDav, Flags};
    use craftcloud_core::storage::{Guard, Mode};

    use super::{Base, Limits, PROBE_DIR, Profile, Request, StoredCheck, Transport, folder_name, guarded, load_check, run, save_check};
    use crate::fake_server::{Answer, FakeServer, Seen};
    use crate::session::Session;

    const NOON: &str = "Tue, 17 Nov 2026 12:00:00 GMT";
    const NEXT_DAY: &str = "Wed, 18 Nov 2026 13:00:00 GMT";

    /// The scripted WebDAV server behind a loopback HTTP server; `drop_at` closes connection
    /// number N (from 1) without an answer.
    fn scripted(flags: Flags, drop_at: Option<usize>) -> (FakeServer, Arc<Mutex<FakeDav>>) {
        let dav = Arc::new(Mutex::new(FakeDav::new("/webdav", flags)));
        let shared = Arc::clone(&dav);
        let server = FakeServer::start_full(move |seen: &Seen| {
            let mut dav = shared.lock().unwrap_or_else(PoisonError::into_inner);
            if drop_at.is_some_and(|n| dav.log.len() + 1 == n) {
                dav.log.push((seen.method.clone(), seen.path.clone(), Vec::new()));
                return Answer::new(Answer::DROP, "");
            }
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        (server, dav)
    }

    fn session(server: &FakeServer) -> Session {
        Session::new(server.server(), Tokens { access_token: "a1".into(), refresh_token: "r1".into() }, Box::new(|_| {}))
    }

    fn base(server: &FakeServer) -> Base {
        Base::parse(&format!("{}/webdav/", server.url)).unwrap()
    }

    /// The usual limits without the settle wait: the scripted server tags every write afresh.
    fn quick() -> Limits {
        Limits { settle: Duration::ZERO, ..Limits::default() }
    }

    fn probe(server: &FakeServer, name: &str) -> (super::Verdict, Vec<Seen>) {
        let verdict = run(&mut session(server), &base(server), name, Profile::default(), quick());
        (verdict, server.seen())
    }

    #[test]
    fn a_well_behaved_server_is_two_way_and_every_request_carries_its_guard() {
        let (server, dav) = scripted(Flags::default(), None);
        let (verdict, seen) = probe(&server, "Laptop-1");
        assert_eq!(verdict.mode, Mode::TwoWay, "{verdict:#?}");
        assert_eq!(verdict.steps.len(), 20);
        for r in &seen {
            assert_eq!(r.bearer(), Some("a1"), "every request is the session's: {r:?}");
            match r.method.as_str() {
                "PUT" => assert!(r.header("if-match").is_some() || r.header("if-none-match") == Some("*"), "an unguarded PUT: {r:?}"),
                "MOVE" | "COPY" => {
                    assert_eq!(r.header("overwrite"), Some("F"), "{r:?}");
                    assert!(r.header("destination").is_some_and(|d| d.contains(&format!("/webdav/{PROBE_DIR}/Laptop-1/"))), "{r:?}");
                }
                "DELETE" => assert!(r.path.starts_with(&format!("/webdav/{PROBE_DIR}/Laptop-1")), "a DELETE outside the probe folder: {r:?}"),
                _ => {}
            }
        }
        assert_eq!(seen.iter().filter(|r| r.method == "DELETE").count(), 2, "the stale-guard delete and the clean-up: {seen:?}");
        assert!(seen.iter().all(|r| r.path == "/webdav/" || r.path.starts_with("/webdav/.craftcloud")), "nothing outside CraftCloud's own folder");
        let dav = dav.lock().unwrap();
        assert!(!dav.has(&format!("{PROBE_DIR}/Laptop-1")), "the probe folder is gone");
        assert!(dav.has(PROBE_DIR), "the probe dir stays for the next check");
    }

    #[test]
    fn misbehaving_servers_get_download_only_or_nothing() {
        let (server, _) = scripted(Flags { ignores_if_none_match: true, ..Flags::default() }, None);
        let (verdict, _) = probe(&server, "Laptop-2");
        assert_eq!(verdict.mode, Mode::DownloadOnly);
        assert_eq!(verdict.capabilities.as_ref().unwrap().create_if_absent, Guard::Ignored);

        let (server, _) = scripted(Flags { ignores_if_match: true, ..Flags::default() }, None);
        assert_eq!(probe(&server, "Laptop-3").0.mode, Mode::DownloadOnly, "2xx to a stale If-Match");

        let (server, _) = scripted(Flags { no_file_etags: true, ..Flags::default() }, None);
        let (verdict, _) = probe(&server, "Laptop-4");
        assert_eq!(verdict.mode, Mode::Refused, "no ETags");
        assert!(verdict.refused_because.as_deref().is_some_and(|w| w.contains("version tag")));
    }

    #[test]
    fn a_server_that_stops_answering_is_refused_and_its_folder_is_swept_next_time() {
        // The connection drops at the 9th request (a PUT in step 4): refused, nothing more sent.
        let (server, dav) = scripted(Flags { date: Some(NOON.into()), ..Flags::default() }, Some(9));
        let (verdict, seen) = probe(&server, "Laptop-5");
        assert_eq!(verdict.mode, Mode::Refused, "{verdict:#?}");
        assert!(verdict.refused_because.as_deref().is_some_and(|w| w.starts_with("no answer at step 4")), "{verdict:#?}");
        assert_eq!(seen.len(), 9);
        assert!(dav.lock().unwrap().has(&format!("{PROBE_DIR}/Laptop-5")), "left behind");
        drop(server);

        // The next day (by the server's clock) another check sweeps it; a folder from an hour ago stays.
        let dav = Arc::try_unwrap(dav).ok().unwrap().into_inner().unwrap();
        let shared = Arc::new(Mutex::new(dav));
        {
            let mut dav = shared.lock().unwrap();
            dav.flags.date = Some(NEXT_DAY.into());
            dav.add_folder(&format!("{PROBE_DIR}/Studio-running"), Some("Wed, 18 Nov 2026 12:30:00 GMT"));
        }
        let again = Arc::clone(&shared);
        let server = FakeServer::start_full(move |seen: &Seen| {
            let mut dav = again.lock().unwrap_or_else(PoisonError::into_inner);
            let resp = dav.answer(&seen.method, &seen.path, &seen.headers, seen.body.as_bytes());
            let mut answer = Answer::new(resp.status, String::from_utf8_lossy(&resp.body).into_owned());
            for (name, value) in resp.headers {
                answer = answer.header(&name, value);
            }
            answer
        });
        let (verdict, seen) = probe(&server, "Laptop-6");
        assert_eq!(verdict.mode, Mode::TwoWay);
        let swept: Vec<&str> = seen.iter().filter(|r| r.method == "DELETE").map(|r| r.path.as_str()).collect();
        assert_eq!(swept[0], format!("/webdav/{PROBE_DIR}/Laptop-5/"), "{swept:?}");
        let dav = shared.lock().unwrap();
        assert!(
            !dav.has(&format!("{PROBE_DIR}/Laptop-5"))
                && dav.has(&format!("{PROBE_DIR}/Studio-running"))
                && !dav.has(&format!("{PROBE_DIR}/Laptop-6"))
        );
    }

    /// A transport straight to the scripted server, no HTTP: for the limits.
    struct Direct {
        dav: FakeDav,
        slow_at: Option<(usize, Duration)>,
        n: usize,
    }

    impl Transport for Direct {
        fn call(&mut self, req: &HttpRequest) -> Result<HttpResponse, String> {
            self.n += 1;
            if let Some((at, by)) = self.slow_at
                && self.n == at
            {
                std::thread::sleep(by);
            }
            Ok(self.dav.answer(req.method.as_str(), &req.url, &req.headers, &req.body))
        }

        fn upload(&mut self, req: &HttpRequest, _: &std::path::Path) -> Result<HttpResponse, String> {
            self.call(req)
        }
    }

    #[test]
    fn a_request_over_the_step_limit_refuses_the_probe() {
        let base = Base::parse("https://cloud.example.org/webdav/").unwrap();
        let mut slow = Direct { dav: FakeDav::new("/webdav", Flags::default()), slow_at: Some((5, Duration::from_millis(60))), n: 0 };
        let limits = Limits { step: Duration::from_millis(20), total: Duration::from_secs(10), settle: Duration::ZERO };
        let verdict = run(&mut slow, &base, "Laptop-7", Profile::default(), limits);
        assert_eq!(verdict.mode, Mode::Refused);
        assert!(verdict.refused_because.as_deref().is_some_and(|w| w.contains("one request took")), "{verdict:#?}");
        assert_eq!(slow.n, 5, "nothing more is sent");

        let mut over = Direct { dav: FakeDav::new("/webdav", Flags::default()), slow_at: Some((3, Duration::from_millis(30))), n: 0 };
        let limits = Limits { step: Duration::from_secs(10), total: Duration::from_millis(10), settle: Duration::ZERO };
        let verdict = run(&mut over, &base, "Laptop-8", Profile::default(), limits);
        assert_eq!(verdict.mode, Mode::Refused);
        assert!(verdict.refused_because.as_deref().is_some_and(|w| w.contains("took longer than")), "{verdict:#?}");

        let mut fine = Direct { dav: FakeDav::new("/webdav", Flags::default()), slow_at: None, n: 0 };
        assert_eq!(run(&mut fine, &base, "Laptop-9", Profile::default(), quick()).mode, Mode::TwoWay);
    }

    /// The two overwrites of the test file wait for a later second (Nextcloud and ownCloud keep
    /// a file's tag within the same second); the step limit doesn't count the wait.
    #[test]
    fn the_overwrites_wait_for_a_later_second() {
        let base = Base::parse("https://cloud.example.org/webdav/").unwrap();
        let mut direct = Direct { dav: FakeDav::new("/webdav", Flags::default()), slow_at: None, n: 0 };
        let limits = Limits { step: Duration::from_millis(40), total: Duration::from_secs(10), settle: Duration::from_millis(60) };
        let started = std::time::Instant::now();
        let verdict = run(&mut direct, &base, "Laptop-11", Profile::default(), limits);
        assert_eq!(verdict.mode, Mode::TwoWay, "{verdict:#?}");
        assert!(started.elapsed() >= Duration::from_millis(120), "two waits of 60 ms: {:?}", started.elapsed());
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn the_runner_refuses_an_unguarded_request_itself() {
        let put = |headers: &[(&str, &str)]| {
            let mut req = Request::new(craftcloud_core::storage::probe::Step::Create, Method::Put, ".craftcloud/probe/x/a.txt", false);
            req.headers = headers.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect();
            req
        };
        let folder = ".craftcloud/probe/x";
        assert!(guarded(&put(&[]), folder).unwrap_err().contains("unguarded PUT"));
        assert!(guarded(&put(&[("If-None-Match", "*")]), folder).is_ok());
        assert!(guarded(&put(&[("If-Match", "\"e1\"")]), folder).is_ok());
        let delete = |path: &str| Request::new(craftcloud_core::storage::probe::Step::CleanUp, Method::Delete, path, true);
        assert!(guarded(&delete(".craftcloud/probe/Laptop-1"), folder).is_ok());
        for outside in [
            "Documents",
            ".craftcloud/versions/a",
            ".craftcloud/probe",
            ".craftcloud/probe/",
            ".craftcloud/probe/../Documents",
            ".craftcloud/probex/a",
        ] {
            assert!(guarded(&delete(outside), folder).is_err(), "{outside}");
        }
        let mut mv = Request::new(craftcloud_core::storage::probe::Step::Move, Method::Move, ".craftcloud/probe/x/a.txt", false);
        mv.destination = Some((".craftcloud/probe/x/c.txt".into(), false));
        assert!(guarded(&mv, folder).unwrap_err().contains("Overwrite"));
        mv.headers.push(("Overwrite".into(), "F".into()));
        assert!(guarded(&mv, folder).is_ok());
        mv.destination = Some(("Documents/c.txt".into(), false));
        assert!(guarded(&mv, folder).unwrap_err().contains("outside"));
    }

    /// On another DAV root (a server's trash bin, which a known-server check reads) the runner
    /// allows the restore of the probe's own trashed file and nothing else: a `MOVE` of a trashed
    /// item to anywhere but its restore address is refused, so is one of an item that didn't come
    /// from the probe's folder, and a `DELETE` there (purging the person's trash, § 16, Q8) always.
    #[test]
    fn on_another_root_only_the_probes_own_restore_may_go() {
        use craftcloud_core::storage::probe::Step;
        let root = "https://cloud.example.org/remote.php/dav/trashbin/john/";
        let folder = ".craftcloud/probe/Laptop-1";
        let restore = |name: &str, to: &str, from: Option<&str>| {
            let mut req = Request::under_root(Step::ServerTrash, Method::Move, root, &format!("trash/{name}"), false).with_header("Overwrite", "F");
            req.destination = Some((to.to_owned(), false));
            req.restores = from.map(str::to_owned);
            req
        };
        let own = Some(".craftcloud/probe/Laptop-1/d.txt");
        assert!(guarded(&restore("d.txt.d1795001745", "restore/d.txt.d1795001745", own), folder).is_ok(), "the allowed restore");
        // Reading is fine.
        assert!(guarded(&Request::under_root(Step::ServerTrash, Method::Propfind, root, "trash", true), folder).is_ok());
        assert!(guarded(&Request::under_root(Step::ServerVersions, Method::Get, root, "trash/d.txt.d1", false), folder).is_ok());
        // A MOVE from the trash bin to anywhere else, or of another item, is refused.
        for (name, to, from, why) in [
            ("d.txt.d1795001745", "Documents/d.txt", own, "not a restore"),
            ("d.txt.d1795001745", "restore/other.d1", own, "another name"),
            ("d.txt.d1795001745", "restore/d.txt.d1795001745", Some("Documents/d.txt"), "not the probe's file"),
            ("d.txt.d1795001745", "restore/d.txt.d1795001745", Some(".craftcloud/probe/Studio-2/d.txt"), "another probe's file"),
            ("d.txt.d1795001745", "restore/d.txt.d1795001745", None, "no origin given"),
            ("sub/d.txt.d1", "restore/sub/d.txt.d1", own, "not a top-level trash item"),
        ] {
            assert!(guarded(&restore(name, to, from), folder).is_err(), "{why}");
        }
        let mut plain = restore("d.txt.d1795001745", "restore/d.txt.d1795001745", own);
        plain.headers.clear();
        assert!(guarded(&plain, folder).is_ok(), "the restore needs no Overwrite header (the server's endpoint, not a name)");
        // Never a DELETE there, nor any other write.
        for method in [Method::Delete, Method::Put, Method::Mkcol, Method::Copy] {
            let req = Request::under_root(Step::ServerTrash, method, root, "trash/d.txt.d1795001745", false)
                .with_header("Overwrite", "F")
                .with_header("If-Match", "\"x\"");
            assert!(guarded(&req, folder).is_err(), "{method:?}");
        }
        assert!(
            guarded(&Request::under_root(Step::ServerTrash, Method::Delete, root, "trash/d.txt.d1", false), folder)
                .unwrap_err()
                .contains("never purged")
        );
    }

    /// ownCloud's trash bin restores by a `MOVE` to a path under the files base, and takes any
    /// path there (measured on 10.15), so the runner allows only the probe's own item back to
    /// exactly where it came from, with `Overwrite: F`: not elsewhere, not another item's place,
    /// not without the header, and never a `DELETE`.
    #[test]
    fn on_an_owncloud_trash_root_only_the_restore_to_where_it_was_may_go() {
        use craftcloud_core::storage::probe::Step;
        let root = "https://cloud.example.org/remote.php/dav/trash-bin/john/";
        let folder = ".craftcloud/probe/Laptop-1";
        let own = ".craftcloud/probe/Laptop-1/d.txt";
        let restore = |id: &str, to: &str, from: Option<&str>, overwrite: Option<&str>| {
            let mut req = Request::under_root(Step::ServerTrash, Method::Move, root, id, false);
            if let Some(o) = overwrite {
                req = req.with_header("Overwrite", o);
            }
            req.destination = Some((to.to_owned(), false));
            req.restores = from.map(str::to_owned);
            req.destination_under_base = true;
            req
        };
        assert!(guarded(&restore("1668", own, Some(own), Some("F")), folder).is_ok(), "the allowed restore");
        let http = restore("1668", own, Some(own), Some("F")).to_http(&Base::parse("https://cloud.example.org/remote.php/dav/files/john/").unwrap());
        assert_eq!(http.url, "https://cloud.example.org/remote.php/dav/trash-bin/john/1668");
        assert_eq!(http.header_value("Destination"), Some("https://cloud.example.org/remote.php/dav/files/john/.craftcloud/probe/Laptop-1/d.txt"));
        for (id, to, from, overwrite, why) in [
            ("1668", ".craftcloud/probe/Laptop-1/elsewhere.txt", Some(own), Some("F"), "elsewhere"),
            ("1668", "Documents/d.txt", Some("Documents/d.txt"), Some("F"), "not the probe's file"),
            ("1668", own, Some(".craftcloud/probe/Studio-2/d.txt"), Some("F"), "another probe's file"),
            ("1668", own, None, Some("F"), "no origin given"),
            ("1668", own, Some(own), None, "no Overwrite: F"),
            ("1668", own, Some(own), Some("T"), "Overwrite: T"),
            ("1668/sub", own, Some(own), Some("F"), "not an item of the root"),
            ("", own, Some(own), Some("F"), "the root itself"),
        ] {
            assert!(guarded(&restore(id, to, from, overwrite), folder).is_err(), "{why}");
        }
        // Reading is fine; a DELETE never.
        assert!(guarded(&Request::under_root(Step::ServerTrash, Method::Propfind, root, "", true), folder).is_ok());
        assert!(guarded(&Request::under_root(Step::ServerTrash, Method::Delete, root, "1668", false), folder).unwrap_err().contains("never purged"));
        // The same request without the hook is read as Nextcloud's shape, which it isn't.
        let mut nc_shaped = restore("1668", own, Some(own), Some("F"));
        nc_shaped.destination_under_base = false;
        assert!(guarded(&nc_shaped, folder).is_err());
    }

    #[test]
    fn the_saved_check_round_trips_in_the_settings_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (server, _) = scripted(Flags::default(), None);
        let (verdict, _) = probe(&server, "Laptop-10");
        let stored = StoredCheck::from_verdict(&verdict, base(&server).url(), "fake 1.0", "2026-11-18T10:00:00Z");
        save_check(dir.path(), &stored).unwrap();
        assert_eq!(load_check(dir.path()), Some(stored.clone()));
        assert!(!dir.path().join("storage-check.json.tmp").exists());
        std::fs::write(dir.path().join("storage-check.json"), "{ broken").unwrap();
        assert_eq!(load_check(dir.path()), None, "a damaged file means a new check, never a guess");
        assert_eq!(load_check(&dir.path().join("missing")), None);
        save_check(&dir.path().join("sub"), &stored).unwrap();
        assert!(load_check(&dir.path().join("sub")).is_some_and(|c| c.mode == Mode::TwoWay && c.measured));
    }

    #[test]
    fn probe_folders_are_named_after_the_device_and_differ() {
        let (a, b) = (folder_name(), folder_name());
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{a}");
        assert!(a.rsplit('-').next().is_some_and(|r| r.len() == 16), "{a}");
    }

    /// The probe against a real server, by hand (never in CI): the WebDAV base, user and app
    /// password from the environment, Basic auth. Prints every step.
    ///
    /// ```text
    /// CRAFTCLOUD_PROBE_URL=http://127.0.0.1:8095/remote.php/dav/files/john/ CRAFTCLOUD_TEST_USER=john \
    /// CRAFTCLOUD_TEST_PASSWORD=… cargo test -p craftcloud-native probe_a_real_server -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a real WebDAV server named in the environment"]
    fn probe_a_real_server() {
        let (Ok(url), Ok(user), Ok(password)) =
            (std::env::var("CRAFTCLOUD_PROBE_URL"), std::env::var("CRAFTCLOUD_TEST_USER"), std::env::var("CRAFTCLOUD_TEST_PASSWORD"))
        else {
            eprintln!("skipped: CRAFTCLOUD_PROBE_URL not set");
            return;
        };
        let base = Base::parse(&url).unwrap();
        let mut transport = crate::session::AppPasswordSession::new(base.clone(), &user, &password).unwrap();
        let verdict = run(&mut transport, &base, &folder_name(), Profile::default(), Limits::default());
        for s in &verdict.steps {
            eprintln!("{:>2}. {} [{}]\n    {}", s.number, s.what, if s.ok { "ok" } else { "NO" }, s.result);
        }
        eprintln!("mode: {}{}", verdict.mode.as_str(), verdict.refused_because.as_deref().map(|w| format!(" ({w})")).unwrap_or_default());
        for c in &verdict.checks {
            eprintln!("  {}: {}", c.check, c.result);
        }
    }
}
