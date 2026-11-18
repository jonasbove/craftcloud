//! The check on connect (docs/WEBDAV-STORAGE.md § 7.5): the steps and the verdict as data and
//! pure functions. Nothing here sends a request; `craftcloud-native` (`storage::probe`) runs it.
//!
//! The probe is a small state machine. [`Probe::next`] gives the request to send now (a method, a
//! cloud path under the WebDAV base, its guard headers, a body), [`Probe::observe`] takes the
//! server's answer and moves on, and [`Probe::verdict`] says what was measured: the
//! [`Capabilities`], the [`Mode`] they allow through [`mode_for`], and every step in plain words.
//! Each step is one of § 7.5's twenty, in order, in `.craftcloud/probe/<device>-<random>/`, so two
//! devices probing at once never meet.
//!
//! What the probe can't take as good news it doesn't: a probe that didn't finish (a transport
//! error, a timeout, a missing answer) is [`Mode::Refused`], never "generic, probably fine"; a
//! version tag (ETag) that doesn't change when the bytes do, or a listing without one, is refused
//! too. Every answer is hostile input: nothing here panics, and listings go through `craft-dav`'s
//! bounded parser.
//!
//! Two steps (16, the server's trash; 17, the server's versions) are known-server checks that a
//! profile supplies ([`ExtraCheck`]); without one the verdict records CraftCloud's own trash and
//! versions, as § 7.3 says.

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

use super::{Capabilities, Checksum, DeleteGuard, FolderEtags, Guard, MissingParent, Mode, Quota, Trash, Versions, mode_for};
use crate::api::dav;
use crate::http::{HttpRequest, HttpResponse, Method};

/// Where probes run on the server: `.craftcloud/probe/<device>-<random>/`.
pub const PROBE_DIR: &str = ".craftcloud/probe";

/// A probe folder older than this, by the server's clock, was left by a crashed probe and is
/// swept (a probe takes seconds).
pub const SWEEP_AFTER_SECS: i64 = 24 * 60 * 60;

/// The made-up version tag the stale-guard steps send: nothing has it.
pub const STALE_ETAG: &str = "\"craftcloud-probe-stale\"";

/// The steps of § 7.5, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Listings,
    ProbeFolder,
    Create,
    CreateAgain,
    ReplaceStale,
    Replace,
    ReplaceSame,
    MissingParent,
    FolderEtags,
    IdsAndChecksums,
    Move,
    MoveOnto,
    Copy,
    Case,
    DeleteStale,
    ServerTrash,
    ServerVersions,
    HardNames,
    DepthInfinity,
    CleanUp,
}

impl Step {
    pub const ALL: [Step; 20] = [
        Step::Listings,
        Step::ProbeFolder,
        Step::Create,
        Step::CreateAgain,
        Step::ReplaceStale,
        Step::Replace,
        Step::ReplaceSame,
        Step::MissingParent,
        Step::FolderEtags,
        Step::IdsAndChecksums,
        Step::Move,
        Step::MoveOnto,
        Step::Copy,
        Step::Case,
        Step::DeleteStale,
        Step::ServerTrash,
        Step::ServerVersions,
        Step::HardNames,
        Step::DepthInfinity,
        Step::CleanUp,
    ];

    /// Its number in § 7.5's table, 1 to 20.
    pub fn number(self) -> u8 {
        Self::ALL.iter().position(|s| *s == self).map_or(0, |i| i as u8 + 1)
    }

    /// What the step checks, in plain words.
    pub const fn what(self) -> &'static str {
        match self {
            Self::Listings => "Listing the server and CraftCloud's own folder",
            Self::ProbeFolder => "Making the probe folder",
            Self::Create => "Creating a file where nothing is (If-None-Match: *) and reading its version tag",
            Self::CreateAgain => "Creating it again where it already is (If-None-Match: *)",
            Self::ReplaceStale => "Overwriting it with a made-up version tag (If-Match)",
            Self::Replace => "Overwriting it with the right version tag (If-Match)",
            Self::ReplaceSame => "Writing the same bytes again",
            Self::MissingParent => "Uploading into a folder that isn't there",
            Self::FolderEtags => "Whether a folder's version tag changes when something in it does",
            Self::IdsAndChecksums => "File ids and checksums in listings",
            Self::Move => "Moving a file (Overwrite: F) and whether its version tag and id survive",
            Self::MoveOnto => "Moving a file onto an existing name (Overwrite: F)",
            Self::Copy => "Copying a file on the server (COPY)",
            Self::Case => "Two names that differ only in case",
            Self::DeleteStale => "Deleting with a made-up version tag (If-Match)",
            Self::ServerTrash => "The server's own trash",
            Self::ServerVersions => "The server's own versions",
            Self::HardNames => "Hard names (a trailing space or dot, a colon, 255 bytes, decomposed accents)",
            Self::DepthInfinity => "Listing a whole tree at once (Depth: infinity)",
            Self::CleanUp => "Deleting the probe folder",
        }
    }
}

/// One request of the probe: what to send, as a cloud path under the WebDAV base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub step: Step,
    pub method: Method,
    /// Relative to the WebDAV base, no leading or trailing slash; `""` is the base itself.
    pub path: String,
    /// Whether `path` is a folder (its URL ends in a slash).
    pub folder: bool,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// For `MOVE` and `COPY`: where to (a cloud path, a folder or not).
    pub destination: Option<(String, bool)>,
    /// This write must land in a later second than the last write to the same file: Nextcloud and
    /// ownCloud keep a file's version tag when its modified time, in whole seconds, is unchanged,
    /// so an overwrite within the same second as the write before it would look like no change.
    /// The runner waits a moment first.
    pub settles: bool,
}

impl Request {
    /// A request of `step` for `path` (a known-server check builds its own this way).
    pub fn new(step: Step, method: Method, path: &str, folder: bool) -> Self {
        Self { step, method, path: path.to_owned(), folder, headers: Vec::new(), body: Vec::new(), destination: None, settles: false }
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    fn body(mut self, body: &[u8]) -> Self {
        self.body = body.to_vec();
        self
    }

    /// The value of a header (case-insensitive name), if set.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Whether this request changes the server (and so must carry its guard).
    pub fn is_write(&self) -> bool {
        matches!(self.method, Method::Put | Method::Move | Method::Copy | Method::Delete | Method::Mkcol)
    }

    /// The HTTP request to send, with every path resolved under `base`.
    pub fn to_http(&self, base: &Base) -> HttpRequest {
        let mut req = HttpRequest::new(self.method, dav::url_under(base.url(), &self.path, self.folder)).body(self.body.clone());
        for (name, value) in &self.headers {
            req = req.header(name, value.clone());
        }
        if let Some((to, folder)) = &self.destination {
            req = req.header("Destination", dav::url_under(base.url(), to, *folder));
        }
        req
    }
}

/// A guarded `PUT`: the only way the probe writes a file.
enum Put {
    /// Only where nothing is (`If-None-Match: *`).
    Create,
    /// Only over this version (`If-Match`).
    Replace(String),
}

/// A WebDAV base (`https://cloud.example.org/remote.php/dav/files/john/`), the URL every probe
/// path hangs under and the path every listed href must start with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Base {
    url: String,
    path: String,
}

impl Base {
    /// `http` or `https` only. The path is kept as given (percent-encoded), without a trailing slash.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parsed = url::Url::parse(text.trim()).map_err(|e| format!("not a WebDAV address: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("not a WebDAV address: {} isn't http or https", parsed.scheme()));
        }
        if parsed.host_str().is_none_or(str::is_empty) {
            return Err("not a WebDAV address: no host".into());
        }
        let path = parsed.path().trim_end_matches('/').to_owned();
        let mut url = parsed.to_string();
        if !url.ends_with('/') {
            url.push('/');
        }
        Ok(Self { url, path })
    }

    /// The base URL, with a trailing slash.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The base's URL path, percent-encoded, without a trailing slash (`/webdav`, `/remote.php/dav/files/john`).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether requests to this base go unencrypted (`http://`).
    pub fn is_plain_http(&self) -> bool {
        self.url.starts_with("http://")
    }

    /// Whether the base is this computer itself (`127.0.0.1`, `::1`, `localhost`): the one place a
    /// password may travel over plain http (docs/WEBDAV-STORAGE.md § 10).
    pub fn is_loopback(&self) -> bool {
        match url::Url::parse(&self.url).ok().and_then(|u| u.host().map(|h| h.to_owned())) {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            None => false,
        }
    }

    /// The server's address without the WebDAV path (`https://cloud.example.org`), what the
    /// settings and the keychain key a sign-in by.
    pub fn origin(&self) -> String {
        url::Url::parse(&self.url).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
    }

    /// The cloud path an href names, relative to this base: `None` when it lies outside it (or
    /// isn't UTF-8), which fails a listing.
    pub fn relative(&self, href: &str) -> Option<String> {
        // An absolute URL is allowed in WebDAV; keep only its path.
        let path = href.find("://").and_then(|i| href[i + 3..].find('/').map(|j| &href[i + 3 + j..])).unwrap_or(href);
        let path = path.split(['?', '#']).next().unwrap_or(path);
        let decoded = percent_decode_str(path).decode_utf8().ok()?;
        let base = percent_decode_str(&self.path).decode_utf8().ok()?;
        let rest = decoded.strip_prefix(base.as_ref())?;
        if !rest.is_empty() && !rest.starts_with('/') {
            return None;
        }
        Some(rest.trim_matches('/').to_owned())
    }
}

/// One entry of a listing the probe reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// Relative to the base; the base itself is `""`.
    pub path: String,
    pub is_folder: bool,
    pub etag: Option<String>,
    /// As the server wrote it (RFC 1123).
    pub modified: Option<String>,
    /// `oc:fileid`, where the server has one.
    pub file_id: Option<String>,
    /// `oc:checksums`, e.g. `SHA1:… MD5:…`.
    pub checksums: Vec<String>,
    /// `getcontentlength`; 0 for a folder or when the server lists none.
    pub size: u64,
    /// `quota-available-bytes`, where the server lists it (a negative number means unknown or
    /// unlimited, as Nextcloud's `-3`).
    pub quota_available: Option<i64>,
}

impl Listed {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }
}

/// What the probe (and the generic driver) ask a listing for.
pub const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:prop><d:resourcetype/><d:getetag/><d:getcontentlength/><d:getlastmodified/><d:quota-available-bytes/><oc:fileid/><oc:checksums/></d:prop></d:propfind>"#;

/// Reads a `PROPFIND` answer. A status other than 207, a body that isn't a multistatus, or an
/// href outside the base is an error (in words).
pub fn parse_listing(resp: &HttpResponse, base: &Base) -> Result<Vec<Listed>, String> {
    if resp.status != 207 {
        return Err(format!("the listing answered {} instead of 207", resp.status));
    }
    let doc = craft_dav::parse_multistatus(&resp.body).map_err(|e| format!("the listing couldn't be read: {e}"))?;
    craft_dav::responses(&doc)
        .map(|response| {
            let href = craft_dav::href(response).ok_or_else(|| "a listing entry without a path".to_owned())?;
            let path = base.relative(&href).ok_or_else(|| format!("a listing named a path outside the server's base: {href}"))?;
            let mut e =
                Listed { path, is_folder: false, etag: None, modified: None, file_id: None, checksums: Vec::new(), size: 0, quota_available: None };
            for prop in craft_dav::properties(response) {
                let text = || Some(prop.text().trim().to_owned()).filter(|t| !t.is_empty());
                match prop.name.as_str() {
                    "d:resourcetype" => e.is_folder = prop.child("d:collection").is_some(),
                    "d:getetag" => e.etag = text(),
                    "d:getcontentlength" => e.size = prop.text().trim().parse().unwrap_or(0),
                    "d:getlastmodified" => e.modified = text(),
                    "d:quota-available-bytes" => e.quota_available = prop.text().trim().parse().ok(),
                    "oc:fileid" => e.file_id = text(),
                    "oc:checksums" => {
                        e.checksums = prop.children("oc:checksum").map(|c| c.text().trim().to_owned()).filter(|c| !c.is_empty()).collect()
                    }
                    _ => {}
                }
            }
            Ok(e)
        })
        .collect()
}

/// Seconds since 1970 of an RFC 1123 time as servers write it (`Date` headers, `getlastmodified`),
/// `None` when it can't be read.
pub fn server_seconds(text: &str) -> Option<i64> {
    jiff::fmt::rfc2822::parse(text.trim()).ok().map(|z| z.timestamp().as_second())
}

/// The probe folders older than [`SWEEP_AFTER_SECS`] **by the server's clock**: each folder's
/// time against the server's `Date` on the same answer (both in seconds), never this computer's
/// clock, so a device whose clock is off can't take another device's running probe for an old
/// one. Without the server's time, or for a folder without one, nothing is swept.
pub fn stale_probe_folders<'a>(folders: impl IntoIterator<Item = (&'a str, Option<i64>)>, server_now: Option<i64>) -> Vec<String> {
    let Some(now) = server_now else { return Vec::new() };
    folders
        .into_iter()
        .filter(|(_, modified)| modified.is_some_and(|m| now.saturating_sub(m) > SWEEP_AFTER_SECS))
        .map(|(path, _)| path.to_owned())
        .collect()
}

/// What the probe has learnt so far, for a known-server check to build on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Learnt {
    /// The probe folder, `.craftcloud/probe/<device>-<random>`.
    pub folder: String,
    /// The version tag the test file (`a.txt`, then `c.txt` after step 11) has now.
    pub etag: Option<String>,
    /// Its id, where the server has ids.
    pub file_id: Option<String>,
    /// Whether `c.txt` is still there (step 15 may have deleted it on a server that ignores the guard).
    pub c_exists: bool,
}

/// A known-server check (step 16, the trash round trip; step 17, the server's versions), which a
/// profile supplies: its requests, one at a time, and whether what it looked for was confirmed.
/// No profile supplies one yet.
pub trait ExtraCheck {
    /// The next request, or `None` when the check is done.
    fn next(&self, learnt: &Learnt) -> Option<Request>;
    /// The answer to the last request.
    fn observe(&mut self, resp: &HttpResponse);
    /// Whether the check found what it looked for (a trashed file that came back whole, one more
    /// version), once done.
    fn confirmed(&self) -> bool;
}

/// What a known server's profile adds to the probe. Both `None` for a server nobody knows.
#[derive(Default)]
pub struct Profile<'a> {
    pub trash: Option<&'a mut dyn ExtraCheck>,
    pub versions: Option<&'a mut dyn ExtraCheck>,
}

/// One row of "What was checked": a check and its result, in plain words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub check: String,
    pub result: String,
}

impl Check {
    fn new(check: &str, result: impl Into<String>) -> Self {
        Self { check: check.to_owned(), result: result.into() }
    }
}

/// What one step of the probe found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    pub step: Step,
    /// 1 to 20.
    pub number: u8,
    /// What was checked.
    pub what: String,
    /// What was found, in plain words.
    pub result: String,
    /// Whether the server did what two-way sync relies on here (a noted-only step is `true`).
    pub ok: bool,
}

/// What the probe found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub mode: Mode,
    /// What was measured; `None` when the probe was refused before it could measure.
    pub capabilities: Option<Capabilities>,
    /// The capability table in plain words ([`checks`]), empty when refused.
    pub checks: Vec<Check>,
    /// Every step done, in order.
    pub steps: Vec<StepResult>,
    /// Why the probe was refused, when it was.
    pub refused_because: Option<String>,
}

/// The capability table in plain words: what `craftcloud server check` prints and Settings shows
/// as "What was checked", for a measured server and for OxiCloud's known capabilities alike.
pub fn checks(c: &Capabilities) -> Vec<Check> {
    let guard = |g: Guard, what: &str| match g {
        Guard::Honoured => format!("honoured: refused (412) when {what}, nothing changed"),
        Guard::Ignored => format!("ignored: the server wrote anyway when {what}"),
    };
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let checksums = match c.checksums {
        Some(Checksum::Sha1) => "SHA1",
        Some(Checksum::Md5) => "MD5",
        Some(Checksum::Adler32) => "Adler-32",
        None => "none",
    };
    vec![
        Check::new("Create only where nothing is (If-None-Match: *)", guard(c.create_if_absent, "a file was already there")),
        Check::new("Overwrite only the version last seen (If-Match)", guard(c.replace_if_match, "the version tag was stale")),
        Check::new(
            "Delete only the version last seen (If-Match)",
            match c.delete_if_match {
                DeleteGuard::Honoured => "honoured: a stale version tag was refused (412)".to_owned(),
                DeleteGuard::Ignored => "ignored or not shown: a fresh listing is checked before every delete instead".to_owned(),
                DeleteGuard::NotUsed => "not used: deletes go to the server's trash over its own API, after a fresh listing".to_owned(),
            },
        ),
        Check::new(
            "A move never lands on an existing name (Overwrite: F)",
            if c.move_no_overwrite { "yes: refused (412), both files unchanged" } else { "no: the move replaced the file" },
        ),
        Check::new("The version tag (ETag) changes on every write", yes_no(c.etag_changes_on_write)),
        Check::new("The version tag survives a move", yes_no(c.etag_kept_by_move)),
        Check::new("An upload's version tag is the one listings show", yes_no(c.put_etag_is_listed)),
        Check::new(
            "A folder's version tag follows its contents",
            match c.folder_etags {
                FolderEtags::Follow => "yes (not relied on: every folder is listed every pass)",
                FolderEtags::DontFollow => "no (every folder is listed every pass)",
                FolderEtags::NotTrusted => "not trusted: seen to miss changes (every folder is listed every pass)",
            },
        ),
        Check::new(
            "An upload into a missing folder answers",
            match c.missing_parent {
                MissingParent::NotFound => "404 (the folders are made first)",
                MissingParent::Conflict => "409 (the folders are made first)",
                MissingParent::Other => "something else (the folders are made first)",
            },
        ),
        Check::new("Listing a whole tree at once (Depth: infinity)", yes_no(c.depth_infinity)),
        Check::new("Copying on the server (COPY)", yes_no(c.copy)),
        Check::new("Files have ids that survive a move", yes_no(c.file_ids)),
        Check::new("Checksums in listings", checksums),
        Check::new("Names that differ only in case", if c.case_insensitive { "one file (case-insensitive)" } else { "two files" }),
        Check::new(
            "Where a delete goes",
            match c.trash {
                Trash::Server => "the server's own trash",
                Trash::Client => "CraftCloud's own trash on the server (.craftcloud/trash/)",
            },
        ),
        Check::new(
            "Where the bytes an overwrite replaces are kept",
            match c.versions {
                Versions::Server => "the server's own versions",
                Versions::Client => "CraftCloud's own versions on the server (.craftcloud/versions/, 5 kept)",
                Versions::None => "nowhere: the server has no versions and can't copy (no two-way sync)",
            },
        ),
        Check::new(
            "Where the free space comes from",
            match c.quota {
                Quota::OxiCloudRest => "the server's account API",
                Quota::Ocs => "the server's OCS user endpoint",
                Quota::WebDavProps => "WebDAV's quota properties",
                Quota::Unknown => "unknown",
            },
        ),
        Check::new("Names the server can't take (left alone by the sync)", names_words(&c.names)),
    ]
}

fn names_words(n: &crate::names::NameRules) -> String {
    let mut parts = Vec::new();
    if n.no_backslash {
        parts.push("a backslash");
    }
    if n.no_control {
        parts.push("a control character");
    }
    if n.max_255_bytes {
        parts.push("more than 255 bytes");
    }
    if parts.is_empty() { "none known".to_owned() } else { parts.join(", ") }
}

/// Where the probe stands: each stage is one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    ListBase,
    ListProbeDir,
    MakeOwn,
    MakeProbeDir,
    Sweep,
    MakeFolder,
    PutA,
    StatA,
    PutAAgain,
    GetA1,
    PutAStale,
    GetA2,
    PutAReplace,
    StatA2,
    PutASame,
    StatA3,
    PutDeep,
    FolderBefore,
    MakeSub,
    MakeDeep,
    PutB,
    FolderAfter,
    ListIds,
    MoveA,
    StatC,
    MoveB,
    StatC2,
    StatB,
    CopyC,
    PutUpper,
    ListCase,
    DeleteC,
    TrashCheck,
    VersionsCheck,
    HardName(usize),
    DepthInf,
    Delete,
    Done,
}

impl Stage {
    fn step(self) -> Step {
        match self {
            Self::ListBase | Self::ListProbeDir | Self::MakeOwn | Self::MakeProbeDir | Self::Sweep => Step::Listings,
            Self::MakeFolder => Step::ProbeFolder,
            Self::PutA | Self::StatA => Step::Create,
            Self::PutAAgain | Self::GetA1 => Step::CreateAgain,
            Self::PutAStale | Self::GetA2 => Step::ReplaceStale,
            Self::PutAReplace | Self::StatA2 => Step::Replace,
            Self::PutASame | Self::StatA3 => Step::ReplaceSame,
            Self::PutDeep => Step::MissingParent,
            Self::FolderBefore | Self::MakeSub | Self::MakeDeep | Self::PutB | Self::FolderAfter => Step::FolderEtags,
            Self::ListIds => Step::IdsAndChecksums,
            Self::MoveA | Self::StatC => Step::Move,
            Self::MoveB | Self::StatC2 | Self::StatB => Step::MoveOnto,
            Self::CopyC => Step::Copy,
            Self::PutUpper | Self::ListCase => Step::Case,
            Self::DeleteC => Step::DeleteStale,
            Self::TrashCheck => Step::ServerTrash,
            Self::VersionsCheck => Step::ServerVersions,
            Self::HardName(_) => Step::HardNames,
            Self::DepthInf => Step::DepthInfinity,
            Self::Delete | Self::Done => Step::CleanUp,
        }
    }
}

/// The hard names of step 18: what each is, and the name itself.
fn hard_names() -> [(&'static str, String); 5] {
    [
        ("a name ending in a space", "space ".to_owned()),
        ("a name ending in a dot", "dot.".to_owned()),
        ("a colon in a name", "colon:name.txt".to_owned()),
        ("a 255-byte name", format!("{}.txt", "a".repeat(251))),
        ("an accented name in decomposed form (NFD)", "e\u{301}.txt".to_owned()),
    ]
}

const ONE: &[u8] = b"one";
const TWO: &[u8] = b"two";
const THREE: &[u8] = b"three";

/// The probe: see the module documentation.
pub struct Probe<'a> {
    base: Base,
    stage: Stage,
    learnt: Learnt,
    caps: Capabilities,
    steps: Vec<StepResult>,
    refused: Option<String>,
    profile: Profile<'a>,
    /// Probe folders still to sweep (step 1).
    sweep: Vec<String>,
    /// The probe folder's version tag before step 9's writes.
    folder_etag_before: Option<Option<String>>,
    /// The version tag the `PUT` of step 3 answered.
    put_etag: Option<String>,
    /// Step 8 made the folders itself (a server that creates missing parents).
    b_exists: bool,
    /// Step 18's answers so far.
    names_said: Vec<String>,
    /// The test file's bytes as last seen (a `GET`), which the next refused write must leave: a
    /// server that ignored step 4's guard has already changed them, which step 5 mustn't count
    /// against its own guard. Capped: anything longer than the probe ever writes counts as changed.
    expected: Vec<u8>,
    /// A yes or no carried from one request of a step to the next (steps 4, 5 and 12).
    scratch: Option<bool>,
}

impl<'a> Probe<'a> {
    /// A probe in `.craftcloud/probe/<folder_name>/` under `base`.
    pub fn new(base: Base, folder_name: &str, profile: Profile<'a>) -> Self {
        let learnt = Learnt { folder: format!("{PROBE_DIR}/{folder_name}"), ..Learnt::default() };
        Self {
            base,
            stage: Stage::ListBase,
            learnt,
            caps: Capabilities::unknown(),
            steps: Vec::new(),
            refused: None,
            profile,
            sweep: Vec::new(),
            folder_etag_before: None,
            put_etag: None,
            b_exists: false,
            names_said: Vec::new(),
            expected: ONE.to_vec(),
            scratch: None,
        }
    }

    /// The probe folder, `.craftcloud/probe/<device>-<random>`.
    pub fn folder(&self) -> &str {
        &self.learnt.folder
    }

    pub fn base(&self) -> &Base {
        &self.base
    }

    /// Whether every request has been sent and answered (or the probe gave up).
    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    fn in_folder(&self, name: &str) -> String {
        format!("{}/{name}", self.learnt.folder)
    }

    fn put(&self, path: &str, guard: &Put, body: &[u8]) -> Request {
        let req = Request::new(self.stage.step(), Method::Put, path, false).body(body);
        match guard {
            Put::Create => req.header("If-None-Match", "*"),
            Put::Replace(etag) => req.header("If-Match", etag.clone()),
        }
    }

    fn propfind(&self, path: &str, folder: bool, depth: &str) -> Request {
        Request::new(self.stage.step(), Method::Propfind, path, folder)
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY.as_bytes())
    }

    fn mkcol(&self, path: &str) -> Request {
        Request::new(self.stage.step(), Method::Mkcol, path, true)
    }

    /// `MOVE` or `COPY`, never onto an existing name.
    fn relocate(&self, method: Method, from: &str, to: &str) -> Request {
        let mut req = Request::new(self.stage.step(), method, from, false).header("Overwrite", "F");
        req.destination = Some((to.to_owned(), false));
        req
    }

    /// The request to send now; `None` once the probe is done.
    pub fn next(&self) -> Option<Request> {
        let folder = &self.learnt.folder;
        let current = || self.learnt.etag.clone().unwrap_or_else(|| STALE_ETAG.to_owned());
        Some(match self.stage {
            Stage::ListBase => self.propfind("", true, "0"),
            Stage::ListProbeDir => self.propfind(PROBE_DIR, true, "1"),
            Stage::MakeOwn => self.mkcol(dav::CRAFTCLOUD_DIR),
            Stage::MakeProbeDir => self.mkcol(PROBE_DIR),
            Stage::Sweep => Request::new(Step::Listings, Method::Delete, self.sweep.last()?, true),
            Stage::MakeFolder => self.mkcol(folder),
            Stage::PutA => self.put(&self.in_folder("a.txt"), &Put::Create, ONE),
            Stage::StatA | Stage::StatA2 | Stage::StatA3 => self.propfind(&self.in_folder("a.txt"), false, "0"),
            Stage::PutAAgain => self.put(&self.in_folder("a.txt"), &Put::Create, TWO),
            Stage::GetA1 | Stage::GetA2 => Request::new(self.stage.step(), Method::Get, &self.in_folder("a.txt"), false),
            Stage::PutAStale => self.put(&self.in_folder("a.txt"), &Put::Replace(STALE_ETAG.to_owned()), TWO),
            // A later second than the write before, or Nextcloud and ownCloud keep the tag (see `Request::settles`).
            Stage::PutAReplace | Stage::PutASame => Request { settles: true, ..self.put(&self.in_folder("a.txt"), &Put::Replace(current()), THREE) },
            Stage::PutDeep | Stage::PutB => self.put(&self.in_folder("sub/deep/b.txt"), &Put::Create, b"b"),
            Stage::FolderBefore | Stage::FolderAfter => self.propfind(folder, true, "0"),
            Stage::MakeSub => self.mkcol(&self.in_folder("sub")),
            Stage::MakeDeep => self.mkcol(&self.in_folder("sub/deep")),
            Stage::ListIds | Stage::ListCase => self.propfind(folder, true, "1"),
            Stage::MoveA => self.relocate(Method::Move, &self.in_folder("a.txt"), &self.in_folder("c.txt")),
            Stage::StatC | Stage::StatC2 => self.propfind(&self.in_folder("c.txt"), false, "0"),
            Stage::MoveB => self.relocate(Method::Move, &self.in_folder("sub/deep/b.txt"), &self.in_folder("c.txt")),
            Stage::StatB => self.propfind(&self.in_folder("sub/deep/b.txt"), false, "0"),
            Stage::CopyC => self.relocate(Method::Copy, &self.in_folder("c.txt"), &self.in_folder("d.txt")),
            Stage::PutUpper => self.put(&self.in_folder("C.TXT"), &Put::Create, b"upper"),
            Stage::DeleteC => Request::new(Step::DeleteStale, Method::Delete, &self.in_folder("c.txt"), false).header("If-Match", STALE_ETAG),
            Stage::TrashCheck => self.profile.trash.as_deref()?.next(&self.learnt)?,
            Stage::VersionsCheck => self.profile.versions.as_deref()?.next(&self.learnt)?,
            Stage::HardName(i) => self.put(&self.in_folder(&hard_names().get(i)?.1), &Put::Create, b"n"),
            Stage::DepthInf => self.propfind(folder, true, "infinity"),
            Stage::Delete => Request::new(Step::CleanUp, Method::Delete, folder, true),
            Stage::Done => return None,
        })
    }

    /// The probe couldn't go on (no answer, a timeout): refused, and nothing more is sent.
    pub fn fail(&mut self, why: &str) {
        let step = self.stage.step();
        self.refuse(step, format!("no answer at step {} ({}): {why}", step.number(), step.what()));
        self.stage = Stage::Done;
    }

    /// A finding that leaves nothing to decide from: the verdict is refused. The probe still
    /// cleans up when the server answers.
    fn refuse(&mut self, step: Step, why: String) {
        self.note(step, false, why.clone());
        if self.refused.is_none() {
            self.refused = Some(why);
        }
        self.stage =
            if self.stage >= Stage::Delete || !self.steps.iter().any(|s| s.step == Step::ProbeFolder && s.ok) { Stage::Done } else { Stage::Delete };
    }

    fn note(&mut self, step: Step, ok: bool, result: impl Into<String>) {
        self.steps.push(StepResult { step, number: step.number(), what: step.what().to_owned(), result: result.into(), ok });
    }

    fn listing(&self, resp: &HttpResponse) -> Result<Vec<Listed>, String> {
        parse_listing(resp, &self.base)
    }

    /// The one entry a depth-0 listing of `path` names.
    fn stat(&self, resp: &HttpResponse, path: &str) -> Result<Listed, String> {
        let entries = self.listing(resp)?;
        entries.into_iter().find(|e| e.path == path).ok_or_else(|| format!("the listing didn't name {path}"))
    }

    /// The server's answer to the request [`Self::next`] gave.
    pub fn observe(&mut self, resp: &HttpResponse) {
        match self.stage.step() {
            Step::Listings => self.on_listings(resp),
            Step::ProbeFolder => self.on_probe_folder(resp),
            Step::Create => self.on_create(resp),
            Step::CreateAgain | Step::ReplaceStale => self.on_stale_guard(resp),
            Step::Replace => self.on_replace(resp),
            Step::ReplaceSame => self.on_replace_same(resp),
            Step::MissingParent => self.on_missing_parent(resp),
            Step::FolderEtags => self.on_folder_etags(resp),
            Step::IdsAndChecksums => self.on_ids(resp),
            Step::Move => self.on_move(resp),
            Step::MoveOnto => self.on_move_onto(resp),
            Step::Copy => self.on_copy(resp),
            Step::Case => self.on_case(resp),
            Step::DeleteStale => self.on_delete_stale(resp),
            Step::ServerTrash | Step::ServerVersions => self.on_extra(resp),
            Step::HardNames => self.on_hard_name(resp),
            Step::DepthInfinity => self.on_depth_infinity(resp),
            Step::CleanUp => self.on_clean_up(resp),
        }
    }

    fn on_listings(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::ListBase => match self.listing(resp) {
                Ok(_) => self.stage = Stage::ListProbeDir,
                Err(why) => self.refuse(Step::Listings, format!("listing the server failed: {why}")),
            },
            Stage::ListProbeDir if resp.status == 404 => self.stage = Stage::MakeOwn,
            Stage::ListProbeDir => match self.listing(resp) {
                Ok(entries) => {
                    let now = resp.header("Date").and_then(server_seconds);
                    let folders = entries
                        .iter()
                        .filter(|e| e.is_folder && e.path != PROBE_DIR && e.path.starts_with(&format!("{PROBE_DIR}/")))
                        .map(|e| (e.path.as_str(), e.modified.as_deref().and_then(server_seconds)));
                    self.sweep = stale_probe_folders(folders, now);
                    self.stage = if self.sweep.is_empty() { Stage::MakeFolder } else { Stage::Sweep };
                }
                Err(why) => self.refuse(Step::Listings, format!("listing {PROBE_DIR} failed: {why}")),
            },
            Stage::MakeOwn | Stage::MakeProbeDir if dav::folder_exists_after(resp) => {
                self.stage = if self.stage == Stage::MakeOwn { Stage::MakeProbeDir } else { Stage::MakeFolder };
            }
            Stage::MakeOwn | Stage::MakeProbeDir => self.refuse(Step::Listings, format!("making CraftCloud's own folder answered {}", resp.status)),
            Stage::Sweep => {
                // Gone or not, the folder isn't tried again this run.
                self.sweep.pop();
                if self.sweep.is_empty() {
                    self.stage = Stage::MakeFolder;
                }
            }
            _ => {}
        }
        if self.stage == Stage::MakeFolder {
            self.note(Step::Listings, true, "the server and CraftCloud's own folder list");
        }
    }

    fn on_probe_folder(&mut self, resp: &HttpResponse) {
        if resp.status == 201 {
            self.note(Step::ProbeFolder, true, format!("made {}", self.learnt.folder));
            self.stage = Stage::PutA;
        } else {
            self.refuse(Step::ProbeFolder, format!("making the probe folder answered {}", resp.status));
        }
    }

    fn on_create(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::PutA if resp.is_success() => {
                self.put_etag = etag_of(resp);
                self.stage = Stage::StatA;
            }
            Stage::PutA => self.refuse(Step::Create, format!("creating a file answered {}", resp.status)),
            _ => match self.stat(resp, &self.in_folder("a.txt")) {
                Ok(Listed { etag: Some(listed), file_id, .. }) => {
                    self.caps.put_etag_is_listed = self.put_etag.as_deref().is_some_and(|put| put.trim_matches('"') == listed.trim_matches('"'));
                    let words = if self.caps.put_etag_is_listed {
                        "created (201) with a version tag, the same the listing shows"
                    } else if self.put_etag.is_some() {
                        "created, but the upload's version tag isn't the one the listing shows (the listing's is used)"
                    } else {
                        "created without a version tag in the answer; the listing has one (it is used)"
                    };
                    self.learnt.etag = Some(listed);
                    self.learnt.file_id = file_id;
                    self.note(Step::Create, true, words);
                    self.stage = Stage::PutAAgain;
                }
                Ok(_) => {
                    self.refuse(Step::Create, "the file is listed without a version tag (ETag), so a change could never be told from a copy".into())
                }
                Err(why) => self.refuse(Step::Create, format!("listing the new file failed: {why}")),
            },
        }
    }

    /// Steps 4 and 5: a guarded write that must be refused, then a `GET` to see the bytes stayed.
    fn on_stale_guard(&mut self, resp: &HttpResponse) {
        let step = self.stage.step();
        match self.stage {
            Stage::PutAAgain | Stage::PutAStale => {
                // Remembered through the step by the ok flag of a provisional result.
                let refused = resp.status == 412;
                self.scratch = Some(refused);
                self.stage = if self.stage == Stage::PutAAgain { Stage::GetA1 } else { Stage::GetA2 };
            }
            _ => {
                let refused = self.scratch.take().unwrap_or(false);
                let unchanged = resp.is_success() && resp.body == self.expected;
                if resp.is_success() {
                    self.expected = if resp.body.len() <= 64 { resp.body.clone() } else { Vec::new() };
                }
                if let Some(etag) = etag_of(resp) {
                    self.learnt.etag = Some(etag);
                }
                let guard = if refused && unchanged { Guard::Honoured } else { Guard::Ignored };
                let words = match (refused, unchanged) {
                    (true, true) => "honoured: refused (412), the bytes unchanged".to_owned(),
                    (true, false) => "ignored: the server said 412 but the bytes changed".to_owned(),
                    (false, _) => "ignored: the server wrote anyway (no 412)".to_owned(),
                };
                if step == Step::CreateAgain {
                    self.caps.create_if_absent = guard;
                    self.stage = Stage::PutAStale;
                } else {
                    self.caps.replace_if_match = guard;
                    self.stage = Stage::PutAReplace;
                }
                self.note(step, guard == Guard::Honoured, words);
            }
        }
    }

    fn on_replace(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::PutAReplace if resp.is_success() => self.stage = Stage::StatA2,
            Stage::PutAReplace => self.refuse(Step::Replace, format!("an overwrite with the right version tag answered {}", resp.status)),
            _ => match self.stat(resp, &self.in_folder("a.txt")) {
                Ok(Listed { etag: Some(new), .. }) => {
                    let old = self.learnt.etag.clone().unwrap_or_default();
                    if new.trim_matches('"') == old.trim_matches('"') {
                        self.refuse(
                            Step::Replace,
                            "the version tag didn't change when the bytes did: a change in the cloud could never be seen".into(),
                        );
                    } else {
                        self.caps.etag_changes_on_write = true;
                        self.learnt.etag = Some(new);
                        self.note(Step::Replace, true, "written, with a new version tag");
                        self.stage = Stage::PutASame;
                    }
                }
                Ok(_) => self.refuse(Step::Replace, "the file is listed without a version tag after an overwrite".into()),
                Err(why) => self.refuse(Step::Replace, format!("listing the overwritten file failed: {why}")),
            },
        }
    }

    fn on_replace_same(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::PutASame if resp.is_success() => self.stage = Stage::StatA3,
            Stage::PutASame => {
                self.note(Step::ReplaceSame, true, format!("answered {} (only noted)", resp.status));
                self.stage = Stage::PutDeep;
            }
            _ => {
                let listed = self.stat(resp, &self.in_folder("a.txt")).ok().and_then(|e| e.etag);
                let changed =
                    listed.as_deref().is_some_and(|new| Some(new.trim_matches('"')) != self.learnt.etag.as_deref().map(|e| e.trim_matches('"')));
                if let Some(etag) = listed {
                    self.learnt.etag = Some(etag);
                }
                let words = if changed {
                    "the version tag changes even for the same bytes (only noted)"
                } else {
                    "the version tag stays for the same bytes (only noted)"
                };
                self.note(Step::ReplaceSame, true, words);
                self.stage = Stage::PutDeep;
            }
        }
    }

    fn on_missing_parent(&mut self, resp: &HttpResponse) {
        let (answer, words) = match resp.status {
            404 => (MissingParent::NotFound, "404 (the folders are made first)".to_owned()),
            409 => (MissingParent::Conflict, "409 (the folders are made first)".to_owned()),
            s if resp.is_success() => {
                self.b_exists = true;
                (MissingParent::Other, format!("{s}: the server made the missing folders itself (the folders are made first anyway)"))
            }
            s => (MissingParent::Other, format!("{s} (the folders are made first)")),
        };
        self.caps.missing_parent = answer;
        self.note(Step::MissingParent, true, words);
        self.stage = Stage::FolderBefore;
    }

    fn on_folder_etags(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::FolderBefore => {
                self.folder_etag_before = Some(self.stat(resp, &self.learnt.folder.clone()).ok().and_then(|e| e.etag));
                self.stage = Stage::MakeSub;
            }
            Stage::MakeSub => self.stage = Stage::MakeDeep,
            Stage::MakeDeep => self.stage = Stage::PutB,
            Stage::PutB => {
                if resp.is_success() || (resp.status == 412 && self.b_exists) {
                    self.b_exists = true;
                }
                self.stage = Stage::FolderAfter;
            }
            _ => {
                let after = self.stat(resp, &self.learnt.folder.clone()).ok().and_then(|e| e.etag);
                let before = self.folder_etag_before.take().flatten();
                let follows = matches!((&before, &after), (Some(b), Some(a)) if b != a);
                self.caps.folder_etags = if follows { FolderEtags::Follow } else { FolderEtags::DontFollow };
                let words = if follows {
                    "yes: the probe folder's version tag changed (not relied on: every folder is listed every pass)"
                } else if before.is_none() || after.is_none() {
                    "folders have no version tag (every folder is listed every pass)"
                } else {
                    "no: the probe folder's version tag stayed (every folder is listed every pass)"
                };
                self.note(Step::FolderEtags, true, words);
                self.stage = Stage::ListIds;
            }
        }
    }

    fn on_ids(&mut self, resp: &HttpResponse) {
        let a = self.in_folder("a.txt");
        let entry = self.listing(resp).ok().and_then(|entries| entries.into_iter().find(|e| e.path == a));
        let (ids, checksums) = match &entry {
            Some(e) => (e.file_id.clone(), e.checksums.clone()),
            None => (None, Vec::new()),
        };
        self.caps.file_ids = ids.is_some();
        self.learnt.file_id = ids;
        self.caps.checksums = checksum_kind(&checksums);
        let words = format!(
            "ids: {}; checksums: {}",
            if self.caps.file_ids { "yes" } else { "none" },
            if checksums.is_empty() { "none".to_owned() } else { checksums.join(" ") }
        );
        self.note(Step::IdsAndChecksums, true, words);
        self.stage = Stage::MoveA;
    }

    fn on_move(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::MoveA if resp.is_success() => {
                self.learnt.c_exists = true;
                self.stage = Stage::StatC;
            }
            Stage::MoveA => {
                self.caps.etag_kept_by_move = false;
                self.note(Step::Move, false, format!("the move answered {}: renames can't be paired by version tag or id", resp.status));
                self.stage = Stage::MoveB;
            }
            _ => {
                let entry = self.stat(resp, &self.in_folder("c.txt")).ok();
                let same_etag = entry
                    .as_ref()
                    .and_then(|e| e.etag.as_deref())
                    .is_some_and(|now| Some(now.trim_matches('"')) == self.learnt.etag.as_deref().map(|e| e.trim_matches('"')));
                let same_id = self.caps.file_ids && entry.as_ref().and_then(|e| e.file_id.as_deref()) == self.learnt.file_id.as_deref();
                self.caps.etag_kept_by_move = same_etag;
                if self.caps.file_ids && !same_id {
                    self.caps.file_ids = false;
                }
                if let Some(e) = entry {
                    if let Some(etag) = e.etag {
                        self.learnt.etag = Some(etag);
                    }
                    self.learnt.file_id = e.file_id;
                }
                let words = format!(
                    "moved (201); the version tag {}{}",
                    if same_etag { "survived" } else { "changed" },
                    if self.caps.file_ids { ", the id survived" } else { "" }
                );
                self.note(Step::Move, true, words);
                self.stage = Stage::MoveB;
            }
        }
    }

    fn on_move_onto(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::MoveB if resp.status == 412 && self.learnt.c_exists => {
                self.scratch = Some(true);
                self.stage = Stage::StatC2;
            }
            Stage::MoveB => {
                self.caps.move_no_overwrite = false;
                let words = if self.learnt.c_exists {
                    format!("no: the move onto an existing name answered {} instead of 412", resp.status)
                } else {
                    format!("couldn't be checked: there was no file to move onto (the move in step 11 failed); answered {}", resp.status)
                };
                self.note(Step::MoveOnto, false, words);
                self.stage = Stage::CopyC;
            }
            Stage::StatC2 => {
                let unchanged = self
                    .stat(resp, &self.in_folder("c.txt"))
                    .ok()
                    .and_then(|e| e.etag)
                    .is_some_and(|now| Some(now.trim_matches('"')) == self.learnt.etag.as_deref().map(|e| e.trim_matches('"')));
                self.scratch = Some(unchanged);
                self.stage = Stage::StatB;
            }
            _ => {
                let c_unchanged = self.scratch.take().unwrap_or(false);
                let b_there = self.stat(resp, &self.in_folder("sub/deep/b.txt")).is_ok();
                self.caps.move_no_overwrite = c_unchanged && b_there;
                let words = if self.caps.move_no_overwrite {
                    "yes: refused (412), both files unchanged".to_owned()
                } else {
                    format!("no: refused with 412, but {}", if c_unchanged { "the moved file is gone" } else { "the target changed" })
                };
                self.note(Step::MoveOnto, self.caps.move_no_overwrite, words);
                self.stage = Stage::CopyC;
            }
        }
    }

    fn on_copy(&mut self, resp: &HttpResponse) {
        self.caps.copy = resp.is_success();
        let words = if self.caps.copy { "yes (201)".to_owned() } else { format!("no: answered {}", resp.status) };
        self.note(Step::Copy, true, words);
        self.stage = Stage::PutUpper;
    }

    fn on_case(&mut self, resp: &HttpResponse) {
        match self.stage {
            Stage::PutUpper if resp.is_success() => self.stage = Stage::ListCase,
            Stage::PutUpper => {
                // 412: C.TXT is c.txt to this server. Anything else can't be told: taken as one file,
                // the side that writes less.
                self.caps.case_insensitive = true;
                let words = if resp.status == 412 {
                    "one file: creating C.TXT beside c.txt was refused (412)".to_owned()
                } else {
                    format!("couldn't be checked (answered {}): taken as one file", resp.status)
                };
                self.note(Step::Case, true, words);
                self.stage = Stage::DeleteC;
            }
            _ => {
                let names: Vec<String> = self.listing(resp).unwrap_or_default().into_iter().map(|e| e.name().to_owned()).collect();
                let two = names.iter().any(|n| n == "c.txt") && names.iter().any(|n| n == "C.TXT");
                self.caps.case_insensitive = !two;
                self.note(Step::Case, true, if two { "two files" } else { "one file: C.TXT replaced c.txt" });
                self.stage = Stage::DeleteC;
            }
        }
    }

    fn on_delete_stale(&mut self, resp: &HttpResponse) {
        let (guard, words) = match resp.status {
            412 => (DeleteGuard::Honoured, "honoured: refused (412)".to_owned()),
            s if resp.is_success() => {
                self.learnt.c_exists = false;
                (DeleteGuard::Ignored, format!("ignored: deleted anyway ({s}); a fresh listing is checked before every delete instead"))
            }
            s => (DeleteGuard::Ignored, format!("not shown: answered {s}; a fresh listing is checked before every delete instead")),
        };
        self.caps.delete_if_match = guard;
        self.note(Step::DeleteStale, true, words);
        self.stage = Stage::TrashCheck;
        self.start_extra();
    }

    /// Steps 16 and 17 run only with a profile; without one they are decided at once.
    fn start_extra(&mut self) {
        loop {
            let has_check = match self.stage {
                Stage::TrashCheck => self.profile.trash.as_ref().is_some_and(|c| c.next(&self.learnt).is_some()),
                Stage::VersionsCheck => self.profile.versions.as_ref().is_some_and(|c| c.next(&self.learnt).is_some()),
                _ => return,
            };
            if has_check {
                return;
            }
            self.finish_extra(None);
        }
    }

    fn finish_extra(&mut self, confirmed: Option<bool>) {
        if self.stage == Stage::TrashCheck {
            let server = confirmed == Some(true);
            self.caps.trash = if server { Trash::Server } else { Trash::Client };
            let words = match confirmed {
                Some(true) => "a deleted file was found in the server's trash and came back whole",
                Some(false) => "the server's trash didn't give the file back: deletes go to CraftCloud's own trash (.craftcloud/trash/)",
                None => "no server trash is known for this kind of server: deletes go to CraftCloud's own trash (.craftcloud/trash/)",
            };
            self.note(Step::ServerTrash, true, words);
            self.stage = Stage::VersionsCheck;
        } else {
            let server = confirmed == Some(true);
            self.caps.versions = if server {
                Versions::Server
            } else if self.caps.copy {
                Versions::Client
            } else {
                Versions::None
            };
            let words = match (confirmed, self.caps.copy) {
                (Some(true), _) => "an overwrite left one more version on the server",
                (Some(false), true) | (None, true) => {
                    "no server versions are known for this kind of server: CraftCloud keeps them (.craftcloud/versions/, 5 kept)"
                }
                (_, false) => "no server versions, and the server can't copy: nowhere to keep the bytes an overwrite replaces (no two-way sync)",
            };
            self.note(Step::ServerVersions, self.caps.versions != Versions::None, words);
            self.stage = Stage::HardName(0);
        }
    }

    fn on_extra(&mut self, resp: &HttpResponse) {
        let check = match self.stage {
            Stage::TrashCheck => self.profile.trash.as_deref_mut(),
            _ => self.profile.versions.as_deref_mut(),
        };
        let Some(check) = check else {
            self.finish_extra(None);
            return self.start_extra();
        };
        check.observe(resp);
        if check.next(&self.learnt).is_none() {
            let confirmed = check.confirmed();
            self.finish_extra(Some(confirmed));
            self.start_extra();
        }
    }

    fn on_hard_name(&mut self, resp: &HttpResponse) {
        let Stage::HardName(i) = self.stage else { return };
        let names = hard_names();
        let what = names.get(i).map_or("a name", |(what, _)| what);
        let said = match resp.status {
            s if resp.is_success() => format!("{what}: accepted ({s})"),
            s @ 400..500 => format!("{what}: refused ({s})"),
            s => format!("{what}: answered {s}"),
        };
        self.names_said.push(said);
        if i + 1 < names.len() {
            self.stage = Stage::HardName(i + 1);
        } else {
            // The rules the sync applies stay the strict ones every server gets (a backslash, a
            // control character, more than 255 bytes); what was seen is recorded in words.
            self.note(Step::HardNames, true, self.names_said.join("; "));
            self.stage = Stage::DepthInf;
        }
    }

    fn on_depth_infinity(&mut self, resp: &HttpResponse) {
        self.caps.depth_infinity = resp.status == 207;
        let words = if self.caps.depth_infinity { "yes (207)".to_owned() } else { format!("no: answered {}", resp.status) };
        self.note(Step::DepthInfinity, true, words);
        self.stage = Stage::Delete;
    }

    fn on_clean_up(&mut self, resp: &HttpResponse) {
        if self.stage == Stage::Delete {
            let gone = resp.is_success() || resp.status == 404;
            let words = if gone { "gone".to_owned() } else { format!("answered {}: left in place, swept by a later check", resp.status) };
            self.note(Step::CleanUp, true, words);
        }
        self.stage = Stage::Done;
    }

    /// What the probe found. Before it is done, or after a failure, the mode is refused.
    pub fn verdict(&self) -> Verdict {
        let refused_because = match (&self.refused, self.stage) {
            (Some(why), _) => Some(why.clone()),
            (None, Stage::Done) => None,
            (None, stage) => Some(format!("the check didn't finish (stopped at step {}, {})", stage.step().number(), stage.step().what())),
        };
        if let Some(why) = refused_because {
            return Verdict { mode: Mode::Refused, capabilities: None, checks: Vec::new(), steps: self.steps.clone(), refused_because: Some(why) };
        }
        let caps = self.caps.clone();
        Verdict { mode: mode_for(&caps), checks: checks(&caps), capabilities: Some(caps), steps: self.steps.clone(), refused_because: None }
    }
}

fn etag_of(resp: &HttpResponse) -> Option<String> {
    resp.header("ETag").map(str::trim).filter(|e| !e.is_empty()).map(str::to_owned)
}

/// The strongest checksum a server lists (`SHA1:…`, `MD5:…`, `ADLER32:…`).
fn checksum_kind(checksums: &[String]) -> Option<Checksum> {
    let kinds: Vec<String> =
        checksums.iter().flat_map(|c| c.split_whitespace()).map(|c| c.split(':').next().unwrap_or("").to_ascii_uppercase()).collect();
    if kinds.iter().any(|k| k == "SHA1") {
        Some(Checksum::Sha1)
    } else if kinds.iter().any(|k| k == "MD5") {
        Some(Checksum::Md5)
    } else if kinds.iter().any(|k| k == "ADLER32") {
        Some(Checksum::Adler32)
    } else {
        None
    }
}

/// The saved result of a check (`storage-check.json` in the settings folder): the date, the
/// server's version string, each check's answer and the mode. Every field has a default, so a
/// later client reads an earlier file; a field the file doesn't name reads as the safe side
/// (refused, nothing measured).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoredCheck {
    /// When the check ran (RFC 3339, this computer's clock; shown, never compared with the server's).
    #[serde(default)]
    pub checked_at: String,
    /// The WebDAV base that was checked.
    #[serde(default)]
    pub server: String,
    /// The server's version string when it ran (a change means a new check).
    #[serde(default)]
    pub server_version: String,
    /// Measured by a probe, or known for the kind of server (OxiCloud).
    #[serde(default)]
    pub measured: bool,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
    /// The capability table in plain words.
    #[serde(default)]
    pub checks: Vec<Check>,
    /// Every step of the probe, in order (empty when nothing was measured).
    #[serde(default)]
    pub steps: Vec<StepResult>,
    #[serde(default)]
    pub refused_because: Option<String>,
}

impl StoredCheck {
    /// A verdict as it is saved.
    pub fn from_verdict(verdict: &Verdict, server: &str, server_version: &str, checked_at: &str) -> Self {
        Self {
            checked_at: checked_at.to_owned(),
            server: server.to_owned(),
            server_version: server_version.to_owned(),
            measured: true,
            mode: verdict.mode,
            capabilities: verdict.capabilities.clone(),
            checks: verdict.checks.clone(),
            steps: verdict.steps.clone(),
            refused_because: verdict.refused_because.clone(),
        }
    }

    /// Reads a saved check; anything unreadable is `None` (a new check runs).
    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned())
    }
}

impl<'a> std::fmt::Debug for Probe<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe")
            .field("folder", &self.learnt.folder)
            .field("stage", &self.stage)
            .field("refused", &self.refused)
            .finish_non_exhaustive()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod fake;

#[cfg(test)]
mod tests;
