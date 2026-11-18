//! The probe's verdict from scripted answers: one test per way a server misbehaves
//! (docs/WEBDAV-STORAGE.md § 7.5's "If not" column and § 13), the sweep rule by the server's
//! clock, the request log's guards, and the saved check's shape.

use proptest::prelude::{ProptestConfig, any, proptest};

use super::fake::{FakeDav, Flags};
use super::{
    Base, Check, ExtraCheck, Learnt, PROBE_DIR, Probe, Profile, Request, STALE_ETAG, Step, StoredCheck, Verdict, checks, parse_listing,
    server_seconds, stale_probe_folders,
};
use crate::http::{HttpResponse, Method};
use crate::names::NameRules;
use crate::storage::{Capabilities, DeleteGuard, FolderEtags, Guard, MissingParent, Mode, Trash, Versions};

const BASE: &str = "https://cloud.example.org/remote.php/dav/files/john/";

fn base() -> Base {
    Base::parse(BASE).unwrap()
}

/// Runs a probe to the end against `server`, handing every request through `send` (which may
/// refuse to answer), and gives the verdict and the requests sent.
fn run_with(server: &mut FakeDav, mut send: impl FnMut(&mut FakeDav, &Request) -> Option<HttpResponse>) -> (Verdict, Vec<Request>) {
    let mut probe = Probe::new(base(), "Laptop-1234", Profile::default());
    let mut sent = Vec::new();
    while let Some(req) = probe.next() {
        sent.push(req.clone());
        match send(server, &req) {
            Some(resp) => probe.observe(&resp),
            None => probe.fail("the connection dropped"),
        }
        assert!(sent.len() < 200, "the probe never ends");
    }
    (probe.verdict(), sent)
}

fn answer(server: &mut FakeDav, req: &Request) -> Option<HttpResponse> {
    let http = req.to_http(&base());
    Some(server.answer(http.method.as_str(), &http.url, &http.headers, &http.body))
}

fn run(flags: Flags) -> (Verdict, Vec<Request>) {
    let mut server = FakeDav::new("/remote.php/dav/files/john", flags);
    run_with(&mut server, answer)
}

fn result_of(verdict: &Verdict, step: Step) -> &str {
    &verdict.steps.iter().find(|s| s.step == step).unwrap_or_else(|| panic!("step {step:?} has no result: {:#?}", verdict.steps)).result
}

#[test]
fn a_well_behaved_server_syncs_both_ways() {
    let (verdict, sent) = run(Flags::default());
    assert_eq!(verdict.mode, Mode::TwoWay, "{verdict:#?}");
    let caps = verdict.capabilities.clone().unwrap();
    assert_eq!((caps.create_if_absent, caps.replace_if_match, caps.delete_if_match), (Guard::Honoured, Guard::Honoured, DeleteGuard::Honoured));
    assert!(caps.move_no_overwrite && caps.etag_changes_on_write && caps.etag_kept_by_move && caps.put_etag_is_listed && caps.copy && caps.file_ids);
    assert_eq!(caps.folder_etags, FolderEtags::Follow);
    assert_eq!(caps.missing_parent, MissingParent::Conflict);
    assert_eq!((caps.trash, caps.versions), (Trash::Client, Versions::Client), "no profile: CraftCloud's own trash and versions");
    assert!(!caps.depth_infinity && !caps.case_insensitive);
    assert_eq!(caps.checksums, Some(crate::storage::Checksum::Sha1));
    assert_eq!(caps.names, NameRules::oxicloud(), "the strict rules, whatever the hard names said");
    // Every step reported, in order, 1 to 20.
    let numbers: Vec<u8> = verdict.steps.iter().map(|s| s.number).collect();
    assert_eq!(numbers, (1..=20).collect::<Vec<u8>>(), "{:#?}", verdict.steps);
    assert!(verdict.steps.iter().all(|s| s.ok), "{:#?}", verdict.steps.iter().filter(|s| !s.ok).collect::<Vec<_>>());
    assert_eq!(verdict.checks.len(), checks(&caps).len());
    assert!(verdict.refused_because.is_none());
    // The folder is gone at the end, and nothing was deleted outside it.
    assert!(sent.last().is_some_and(|r| r.method == Method::Delete && r.path == format!("{PROBE_DIR}/Laptop-1234")));
}

/// Every write the probe sends carries its guard: it is a probe of guards.
#[test]
fn every_write_carries_its_guard() {
    let (_, sent) = run(Flags::default());
    for req in &sent {
        match req.method {
            Method::Put => {
                assert!(req.header_value("If-None-Match") == Some("*") || req.header_value("If-Match").is_some(), "a PUT without a guard: {req:?}")
            }
            Method::Move | Method::Copy => {
                assert_eq!(req.header_value("Overwrite"), Some("F"), "{req:?}");
                assert!(req.destination.as_ref().is_some_and(|(to, _)| to.starts_with(&format!("{PROBE_DIR}/"))), "{req:?}");
            }
            Method::Delete => assert!(req.path.starts_with(&format!("{PROBE_DIR}/")), "a DELETE outside the probe folder: {req:?}"),
            _ => {}
        }
        assert!(req.path.is_empty() || req.path.starts_with(".craftcloud"), "a request outside CraftCloud's own folder: {req:?}");
    }
    // The two overwrites of the same file must land in a later second than the write before them
    // (Nextcloud and ownCloud keep the tag otherwise); nothing else waits.
    let settling: Vec<Step> = sent.iter().filter(|r| r.settles).map(|r| r.step).collect();
    assert_eq!(settling, [Step::Replace, Step::ReplaceSame]);
    let stale: Vec<&Request> = sent.iter().filter(|r| r.header_value("If-Match") == Some(STALE_ETAG)).collect();
    assert_eq!(stale.len(), 2, "a stale PUT and a stale DELETE: {stale:?}");
    assert!(sent.iter().any(|r| r.method == Method::Propfind && r.header_value("Depth") == Some("infinity")));
}

#[test]
fn a_server_that_ignores_if_none_match_is_download_only() {
    let (verdict, _) = run(Flags { ignores_if_none_match: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::DownloadOnly);
    let caps = verdict.capabilities.clone().unwrap();
    assert_eq!(caps.create_if_absent, Guard::Ignored);
    assert!(result_of(&verdict, Step::CreateAgain).starts_with("ignored"), "{verdict:#?}");
    // The create that was wrongly written changed the bytes; step 5's guard, which held, isn't
    // blamed for that (OpenCloud: If-None-Match ignored, If-Match honoured).
    assert_eq!(caps.replace_if_match, Guard::Honoured, "{verdict:#?}");
    assert!(result_of(&verdict, Step::ReplaceStale).starts_with("honoured"), "{verdict:#?}");
}

#[test]
fn a_2xx_to_a_stale_if_match_is_download_only() {
    let (verdict, _) = run(Flags { ignores_if_match: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::DownloadOnly);
    assert_eq!(verdict.capabilities.clone().unwrap().replace_if_match, Guard::Ignored);
    assert!(result_of(&verdict, Step::ReplaceStale).starts_with("ignored"));
}

#[test]
fn a_move_that_overwrites_is_download_only() {
    let (verdict, _) = run(Flags { move_overwrites: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::DownloadOnly);
    assert!(!verdict.capabilities.clone().unwrap().move_no_overwrite);
    assert!(result_of(&verdict, Step::MoveOnto).starts_with("no:"), "{verdict:#?}");
}

#[test]
fn an_etag_that_doesnt_change_with_the_bytes_is_refused() {
    let (verdict, sent) = run(Flags { fixed_etags: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::Refused);
    assert!(verdict.capabilities.is_none() && verdict.checks.is_empty());
    assert!(verdict.refused_because.as_deref().is_some_and(|w| w.contains("version tag didn't change")), "{verdict:#?}");
    // The server answers, so the probe still cleans up after itself.
    assert!(sent.last().is_some_and(|r| r.method == Method::Delete && r.step == Step::CleanUp), "{:?}", sent.last());
}

#[test]
fn listings_without_etags_are_refused() {
    let (verdict, _) = run(Flags { no_file_etags: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::Refused);
    assert!(verdict.refused_because.as_deref().is_some_and(|w| w.contains("without a version tag")), "{verdict:#?}");
}

#[test]
fn a_probe_that_doesnt_finish_is_refused() {
    // The server stops answering at the 9th request.
    let mut server = FakeDav::new("/remote.php/dav/files/john", Flags::default());
    let mut n = 0;
    let (verdict, sent) = run_with(&mut server, |server, req| {
        n += 1;
        if n == 9 { None } else { answer(server, req) }
    });
    assert_eq!(verdict.mode, Mode::Refused);
    assert_eq!(sent.len(), 9, "nothing more is sent once the server stopped answering");
    assert!(verdict.refused_because.as_deref().is_some_and(|w| w.starts_with("no answer at step")), "{verdict:#?}");
    assert!(server.has(&format!("{PROBE_DIR}/Laptop-1234")), "the probe folder is left for the next check to sweep");

    // A verdict asked for before the end is refused too: never "probably fine".
    let mut early = Probe::new(base(), "Laptop-1", Profile::default());
    assert_eq!(early.verdict().mode, Mode::Refused);
    let req = early.next().unwrap();
    assert_eq!((req.method, req.path.as_str(), req.header_value("Depth")), (Method::Propfind, "", Some("0")));
    early.observe(&server.answer("PROPFIND", &req.to_http(&base()).url, &req.to_http(&base()).headers, &[]));
    assert_eq!(early.verdict().mode, Mode::Refused);
    assert!(early.verdict().refused_because.unwrap().contains("didn't finish"));
}

#[test]
fn no_copy_and_no_server_versions_is_download_only() {
    // Q2: nowhere to keep the bytes an overwrite replaces.
    let (verdict, _) = run(Flags { no_copy: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::DownloadOnly);
    let caps = verdict.capabilities.clone().unwrap();
    assert!(!caps.copy);
    assert_eq!(caps.versions, Versions::None);
    assert!(result_of(&verdict, Step::ServerVersions).contains("no two-way sync"));
}

#[test]
fn a_delete_that_ignores_if_match_still_syncs_both_ways() {
    let (verdict, _) = run(Flags { delete_ignores_if_match: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::TwoWay);
    assert_eq!(verdict.capabilities.clone().unwrap().delete_if_match, DeleteGuard::Ignored);
}

#[test]
fn what_is_only_recorded_doesnt_change_the_mode() {
    let flags = Flags {
        depth_infinity: true,
        missing_parent_404: true,
        folder_etags_stay: true,
        case_insensitive: true,
        no_ids: true,
        move_changes_etag: true,
        ..Flags::default()
    };
    let (verdict, _) = run(flags);
    assert_eq!(verdict.mode, Mode::TwoWay, "{verdict:#?}");
    let caps = verdict.capabilities.clone().unwrap();
    assert!(caps.depth_infinity && caps.case_insensitive && !caps.file_ids && !caps.etag_kept_by_move);
    assert_eq!((caps.missing_parent, caps.folder_etags, caps.checksums), (MissingParent::NotFound, FolderEtags::DontFollow, None));
    assert!(result_of(&verdict, Step::Case).starts_with("one file"));
}

#[test]
fn a_server_without_move_is_download_only_and_says_why() {
    let (verdict, _) = run(Flags { no_move: true, ..Flags::default() });
    assert_eq!(verdict.mode, Mode::DownloadOnly);
    assert!(result_of(&verdict, Step::Move).contains("405"));
    assert!(result_of(&verdict, Step::MoveOnto).starts_with("couldn't be checked"), "{verdict:#?}");
}

#[test]
fn hard_names_are_recorded_in_words_and_the_rules_stay_strict() {
    let (verdict, sent) = run(Flags::default());
    let words = result_of(&verdict, Step::HardNames);
    for what in ["a name ending in a space", "a name ending in a dot", "a colon in a name", "a 255-byte name", "decomposed form"] {
        assert!(words.contains(what), "{words}");
    }
    assert!(words.contains("accepted"), "{words}");
    let names: Vec<&str> = sent.iter().filter(|r| r.step == Step::HardNames).map(|r| r.path.rsplit('/').next().unwrap_or("")).collect();
    assert_eq!(names.len(), 5);
    assert!(names.iter().any(|n| n.len() == 255), "{names:?}");
    assert!(names.iter().any(|n| n.ends_with(' ')) && names.iter().any(|n| n.ends_with('.')) && names.iter().any(|n| n.contains(':')));
    assert_eq!(verdict.capabilities.clone().unwrap().names, NameRules::oxicloud());
}

/// Steps 16 and 17 are a profile's: a check that confirms records the server's trash and
/// versions, one that doesn't leaves CraftCloud's own.
#[test]
fn a_profile_supplies_the_trash_and_versions_checks() {
    struct Stub {
        sent: usize,
        confirm: bool,
    }
    impl ExtraCheck for Stub {
        fn next(&self, learnt: &Learnt) -> Option<Request> {
            (self.sent < 2).then(|| {
                let mut req = Request::new(Step::ServerTrash, Method::Propfind, &format!("{}/c.txt", learnt.folder), false);
                req.headers.push(("Depth".into(), "0".into()));
                req
            })
        }
        fn observe(&mut self, resp: &HttpResponse, learnt: &Learnt) {
            assert_eq!(resp.status, 207);
            assert!(learnt.folder.starts_with(PROBE_DIR));
            self.sent += 1;
        }
        fn confirmed(&self) -> bool {
            self.confirm
        }
    }
    for confirm in [true, false] {
        let mut trash = Stub { sent: 0, confirm };
        let mut versions = Stub { sent: 0, confirm };
        let mut server = FakeDav::new("/remote.php/dav/files/john", Flags::default());
        let mut probe = Probe::new(base(), "Studio-7", Profile { trash: Some(&mut trash), versions: Some(&mut versions), lowered: None });
        let mut extra = 0;
        while let Some(req) = probe.next() {
            extra += usize::from(matches!(req.step, Step::ServerTrash | Step::ServerVersions));
            let http = req.to_http(&base());
            probe.observe(&server.answer(http.method.as_str(), &http.url, &http.headers, &http.body));
        }
        assert_eq!(extra, 4, "two requests per check");
        let verdict = probe.verdict();
        assert_eq!(verdict.mode, Mode::TwoWay);
        let caps = verdict.capabilities.clone().unwrap();
        let expected = if confirm { (Trash::Server, Versions::Server) } else { (Trash::Client, Versions::Client) };
        assert_eq!((caps.trash, caps.versions), expected);
    }
}

// --- The sweep: by the server's clock only -------------------------------------------------------

const NOON: &str = "Tue, 17 Nov 2026 12:00:00 GMT";

#[test]
fn old_probe_folders_are_swept_by_the_servers_clock_only() {
    let now = server_seconds(NOON);
    let hour_old = server_seconds("Tue, 17 Nov 2026 11:00:00 GMT");
    let day_old = server_seconds("Mon, 16 Nov 2026 11:59:00 GMT");
    let exactly_a_day = server_seconds("Mon, 16 Nov 2026 12:00:00 GMT");
    assert!(now.is_some() && hour_old.is_some() && day_old.is_some());
    let folders = || [("p/Laptop-1", hour_old), ("p/Studio-2", day_old), ("p/Studio-3", None), ("p/Studio-4", exactly_a_day)];
    assert_eq!(stale_probe_folders(folders(), now), ["p/Studio-2"], "only the folder older than a day goes; one without a time stays");
    assert!(stale_probe_folders(folders(), None).is_empty(), "no Date header: nothing is swept");
    // A server whose clock is a year behind this computer's: still only its own clock counts.
    let server_in_the_past = server_seconds("Tue, 18 Nov 2025 12:00:00 GMT");
    assert!(stale_probe_folders(folders(), server_in_the_past).is_empty(), "by that clock every folder is in the future");
    assert_eq!(server_seconds("not a time"), None);
}

#[test]
fn older_than_takes_any_span_by_the_servers_clock() {
    use super::older_than;
    let now = server_seconds("Mon, 15 Jun 2026 12:00:00 GMT");
    let two_days_old = server_seconds("Sat, 13 Jun 2026 12:00:00 GMT");
    let month_old = server_seconds("Wed, 13 May 2026 11:00:00 GMT");
    let entries = || [("a", two_days_old), ("b", month_old), ("c", None)];
    let thirty_days = 30 * 24 * 60 * 60;
    assert_eq!(older_than(entries(), now, thirty_days), ["b"], "only the one past 30 days, and only with a time");
    assert!(older_than(entries(), None, thirty_days).is_empty(), "no server time: nothing");
    assert_eq!(older_than(entries(), now, 24 * 60 * 60).len(), 2, "a one-day span catches both the two-day-old and the month-old");
}

#[test]
fn the_probe_sweeps_only_old_folders_and_only_inside_its_own_folder() {
    let mut server = FakeDav::new("/remote.php/dav/files/john", Flags { date: Some(NOON.into()), ..Flags::default() });
    server.add_folder(&format!("{PROBE_DIR}/Studio-old"), Some("Mon, 16 Nov 2026 11:00:00 GMT"));
    server.add_folder(&format!("{PROBE_DIR}/Studio-fresh"), Some("Tue, 17 Nov 2026 11:30:00 GMT"));
    server.add_folder(&format!("{PROBE_DIR}/Studio-undated"), None);
    server.add_folder("Documents", Some("Mon, 16 Nov 2026 11:00:00 GMT"));
    let (verdict, sent) = run_with(&mut server, answer);
    assert_eq!(verdict.mode, Mode::TwoWay);
    let swept: Vec<&str> = sent.iter().filter(|r| r.method == Method::Delete && r.step == Step::Listings).map(|r| r.path.as_str()).collect();
    assert_eq!(swept, [format!("{PROBE_DIR}/Studio-old")]);
    assert!(!server.has(&format!("{PROBE_DIR}/Studio-old")));
    assert!(server.has(&format!("{PROBE_DIR}/Studio-fresh")) && server.has(&format!("{PROBE_DIR}/Studio-undated")) && server.has("Documents"));

    // Without a Date header the same listing sweeps nothing, however old the folders look.
    let mut server = FakeDav::new("/remote.php/dav/files/john", Flags::default());
    server.add_folder(&format!("{PROBE_DIR}/Studio-old"), Some("Mon, 16 Nov 2020 11:00:00 GMT"));
    let (_, sent) = run_with(&mut server, answer);
    assert!(!sent.iter().any(|r| r.method == Method::Delete && r.step == Step::Listings), "{sent:?}");
    assert!(server.has(&format!("{PROBE_DIR}/Studio-old")));
}

#[test]
fn a_server_without_craftclouds_folder_gets_one() {
    let (verdict, sent) = run(Flags::default());
    assert_eq!(verdict.mode, Mode::TwoWay);
    let made: Vec<&str> = sent.iter().filter(|r| r.method == Method::Mkcol).map(|r| r.path.as_str()).collect();
    assert_eq!(made[..3], [".craftcloud", PROBE_DIR, &format!("{PROBE_DIR}/Laptop-1234")]);
}

// --- Listings and the base ---------------------------------------------------------------------------

#[test]
fn hrefs_are_read_against_the_base_and_never_outside_it() {
    let b = base();
    assert_eq!(b.url(), BASE);
    assert_eq!(b.path(), "/remote.php/dav/files/john");
    assert_eq!(b.relative("/remote.php/dav/files/john/Documents/My%20notes.md").as_deref(), Some("Documents/My notes.md"));
    assert_eq!(b.relative("/remote.php/dav/files/john/").as_deref(), Some(""));
    assert_eq!(b.relative("https://cloud.example.org/remote.php/dav/files/john/a.txt").as_deref(), Some("a.txt"));
    assert_eq!(b.relative("/remote.php/dav/files/jane/a.txt"), None, "another user's files");
    assert_eq!(b.relative("/remote.php/dav/files/johnny/a.txt"), None, "a longer name that starts the same");
    assert_eq!(b.relative("/webdav/a.txt"), None);
    let oxi = Base::parse("http://127.0.0.1:8093").unwrap();
    assert_eq!((oxi.url(), oxi.path()), ("http://127.0.0.1:8093/", ""));
    assert_eq!(oxi.relative("/a.txt").as_deref(), Some("a.txt"));
    assert!(Base::parse("ftp://x/").is_err() && Base::parse("not a url").is_err() && Base::parse("http:///").is_err());

    let mut server = FakeDav::new("/remote.php/dav/files/john", Flags::default());
    server.add_folder("Docs", None);
    server.add_file("Docs/a.txt", b"x");
    let headers = vec![("Depth".to_owned(), "1".to_owned())];
    let listed = parse_listing(&server.answer("PROPFIND", "/remote.php/dav/files/john/Docs/", &headers, &[]), &b).unwrap();
    assert_eq!(listed.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["Docs", "Docs/a.txt"]);
    assert!(listed[1].file_id.is_some() && listed[1].checksums[0].starts_with("SHA1:"));
    let outside = HttpResponse::new(
        207,
        r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/remote.php/dav/files/jane/x</d:href></d:response></d:multistatus>"#,
    );
    assert!(parse_listing(&outside, &b).unwrap_err().contains("outside the server's base"));
    assert!(parse_listing(&HttpResponse::new(200, "<html/>"), &b).is_err());
}

#[test]
fn requests_resolve_under_the_base() {
    let probe = Probe::new(base(), "Laptop-1", Profile::default());
    let http = probe.next().unwrap().to_http(probe.base());
    assert_eq!(http.url, BASE);
    let mut req = Request::new(Step::Move, Method::Move, ".craftcloud/probe/Laptop-1/a b.txt", false);
    req.destination = Some((".craftcloud/probe/Laptop-1/c.txt".into(), false));
    let http = req.to_http(&base());
    assert_eq!(http.url, format!("{BASE}.craftcloud/probe/Laptop-1/a%20b.txt"));
    assert_eq!(http.header_value("Destination"), Some(format!("{BASE}.craftcloud/probe/Laptop-1/c.txt").as_str()));
    assert!(req.is_write());
}

/// A known-server check's request on another DAV root (a trash bin) resolves under that root,
/// its destination too, never under the files base.
#[test]
fn a_request_under_another_root_resolves_there() {
    let root = "https://cloud.example.org/remote.php/dav/trashbin/john";
    let mut req = Request::under_root(Step::ServerTrash, Method::Move, root, "trash/d.txt.d1795001745", false).with_header("Overwrite", "F");
    req.destination = Some(("restore/d.txt.d1795001745".into(), false));
    req.restores = Some(".craftcloud/probe/Laptop-1/d.txt".into());
    let http = req.to_http(&base());
    assert_eq!(http.url, format!("{root}/trash/d.txt.d1795001745"));
    assert_eq!(http.header_value("Destination"), Some(format!("{root}/restore/d.txt.d1795001745").as_str()));
    assert_eq!(req.root.as_deref(), Some(format!("{root}/").as_str()), "a trailing slash is added");
    let get = Request::under_root(Step::ServerVersions, Method::Get, &format!("{root}/"), "a b", false).with_body(b"x");
    assert_eq!(get.to_http(&base()).url, format!("{root}/a%20b"));
    assert_eq!(get.body, b"x");
}

/// The scripted server's same-second tag reuse and client-sent checksums (what the Nextcloud
/// profile's checksum check is tested against, docs/WEBDAV-STORAGE.md § 16, Q13).
#[test]
fn the_scripted_server_can_reuse_a_tag_within_a_second_and_list_sent_checksums() {
    let mut server = FakeDav::new("/dav", Flags { etag_reused_within_second: true, checksums_from_client_only: true, ..Flags::default() });
    let headers = |h: &[(&str, &str)]| h.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect::<Vec<_>>();
    let first = server.answer("PUT", "/dav/a.txt", &headers(&[("If-None-Match", "*"), ("OC-Checksum", "SHA1:aaaa")]), b"one");
    let e1 = first.header("ETag").unwrap().to_owned();
    let second = server.answer("PUT", "/dav/a.txt", &headers(&[("If-Match", &e1)]), b"two");
    assert_eq!(second.header("ETag"), Some(e1.as_str()), "within the same second: the tag is kept");
    assert_eq!(server.checksum("a.txt"), None, "a PUT without OC-Checksum clears it");
    let listing = server.answer("PROPFIND", "/dav/a.txt", &headers(&[("Depth", "0")]), b"");
    assert!(!String::from_utf8_lossy(&listing.body).contains("oc:checksum>"), "nothing listed without one");
    let third = server.answer("PUT", "/dav/a.txt", &headers(&[("If-Match", &e1), ("OC-Checksum", "SHA1:bbbb")]), b"three");
    assert!(third.is_success());
    let listing = server.answer("PROPFIND", "/dav/a.txt", &headers(&[("Depth", "0")]), b"");
    assert!(String::from_utf8_lossy(&listing.body).contains("<oc:checksum>SHA1:bbbb</oc:checksum>"));
    // Another client's write right behind ours: the same tag, other bytes, no checksum.
    server.flags.other_writes_after_put = Some(b"theirs".to_vec());
    let fourth = server.answer("PUT", "/dav/a.txt", &headers(&[("If-Match", &e1), ("OC-Checksum", "SHA1:cccc")]), b"four");
    assert_eq!(fourth.header("ETag"), Some(e1.as_str()));
    assert_eq!(server.body("a.txt"), Some(b"theirs".as_slice()));
    assert_eq!(server.checksum("a.txt"), None);
}

// --- Words and the saved check ------------------------------------------------------------------

#[test]
fn oxiclouds_known_capabilities_in_words() {
    let rows = checks(&Capabilities::oxicloud());
    assert_eq!(rows.len(), 18);
    let find = |what: &str| rows.iter().find(|c| c.check.starts_with(what)).map(|c| c.result.as_str()).unwrap_or_else(|| panic!("{what}"));
    assert!(find("Create only").starts_with("honoured"));
    assert!(find("Delete only").starts_with("not used"));
    assert!(find("A folder's version tag").starts_with("not trusted"));
    assert_eq!(find("Where a delete goes"), "the server's own trash");
    assert!(find("Where the bytes").contains(".craftcloud/versions/"));
    assert_eq!(find("Names the server can't take"), "a backslash, a control character, more than 255 bytes");
    assert_eq!((Mode::TwoWay.as_str(), Mode::DownloadOnly.as_str(), Mode::Refused.as_str()), ("two_way", "download_only", "refused"));
    assert_eq!(Mode::TwoWay.words(), "two-way");
    assert_eq!(Step::CleanUp.number(), 20);
    assert_eq!(Step::Listings.number(), 1);
}

#[test]
fn a_saved_check_round_trips_and_an_older_file_reads_safely() {
    let (verdict, _) = run(Flags::default());
    let stored = StoredCheck::from_verdict(&verdict, BASE, "31.0.1", "2026-11-18T10:00:00Z");
    let json = stored.to_json();
    assert_eq!(StoredCheck::from_json(json.as_bytes()).unwrap(), stored);
    assert!(json.contains("\"mode\": \"two_way\"") && json.contains("\"create_if_absent\": \"honoured\""), "{json}");
    assert!(json.contains("\"checked_at\": \"2026-11-18T10:00:00Z\"") && json.contains("\"server_version\": \"31.0.1\""));
    assert!(stored.steps.len() == 20 && stored.measured);

    // A file an earlier (or later) client wrote with fewer fields: nothing is taken as more than it says.
    let old = StoredCheck::from_json(br#"{"checked_at": "2026-11-18T10:00:00Z", "checks": [{"check": "x", "result": "y"}]}"#).unwrap();
    assert_eq!(old.mode, Mode::Refused);
    assert!(old.capabilities.is_none() && !old.measured && old.steps.is_empty());
    assert_eq!(old.checks, [Check { check: "x".into(), result: "y".into() }]);
    // Capabilities a file names in part read as the unknown server plus what it says; names a
    // file leaves out read as the strict rules.
    let partial = StoredCheck::from_json(br#"{"mode": "two_way", "capabilities": {"create_if_absent": "honoured", "names": {}}}"#).unwrap();
    let caps = partial.capabilities.unwrap();
    assert_eq!(caps.create_if_absent, Guard::Honoured);
    assert_eq!(caps.replace_if_match, Guard::Ignored);
    assert_eq!(caps.names, NameRules::oxicloud());
    assert_eq!(crate::storage::mode_for(&caps), Mode::Refused, "what the file says decides, never its mode field: the rest is unknown");
    assert!(StoredCheck::from_json(b"not json").is_none());
}

fn rank(mode: Mode) -> u8 {
    match mode {
        Mode::Refused => 0,
        Mode::DownloadOnly => 1,
        Mode::TwoWay => 2,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// One more thing going wrong never raises the mode.
    #[test]
    fn an_extra_failing_step_never_raises_the_mode(
        ignores_if_none_match in any::<bool>(), ignores_if_match in any::<bool>(), move_overwrites in any::<bool>(),
        no_copy in any::<bool>(), delete_ignores_if_match in any::<bool>(), no_ids in any::<bool>(), extra in 0u8..6,
    ) {
        let flags = Flags { ignores_if_none_match, ignores_if_match, move_overwrites, no_copy, delete_ignores_if_match, no_ids, ..Flags::default() };
        let (before, _) = run(flags.clone());
        let worse = match extra {
            0 => Flags { ignores_if_none_match: true, ..flags },
            1 => Flags { ignores_if_match: true, ..flags },
            2 => Flags { move_overwrites: true, ..flags },
            3 => Flags { no_copy: true, ..flags },
            4 => Flags { fixed_etags: true, ..flags },
            _ => Flags { no_file_etags: true, ..flags },
        };
        let (after, _) = run(worse);
        proptest::prop_assert!(rank(after.mode) <= rank(before.mode), "{:?} -> {:?}", before.mode, after.mode);
    }
}
