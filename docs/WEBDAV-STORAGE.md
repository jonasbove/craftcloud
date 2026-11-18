# Any WebDAV server as the storage: design

Status: **design, decisions taken** (milestone M9, planned for 0.3.0, on `main` behind an opt-in until the
release; section 16). Today the desktop client and the `craftcloud` command line sync only with CraftCloud's own
server (OxiCloud).
This document says how they can sync with other WebDAV servers (Nextcloud, ownCloud, OpenCloud, a NAS …) without
ever making the sync less safe. Which mode each server gets is measured, not promised: OpenCloud and
`rclone serve webdav`, for example, get download only (section 16).

It builds on the engine described in [ARCHITECTURE.md › Sync](ARCHITECTURE.md#sync) and on the sync
non-negotiables in [AGENTS.md](../AGENTS.md), which bind every part of it. Code references are to
`crates/core/src/{reconcile,names,sync}.rs`, `crates/core/src/api/dav.rs` and
`crates/native/src/{sync,session,http,secrets}.rs`.

The facts about the other servers below were **measured on 2026-11-16**: by hand with `curl` against Nextcloud 31,
ownCloud 10.15 and OpenCloud (rolling), and with a throwaway spike branch that made today's engine talk plain
WebDAV to all three and ran the live sync suite against them. The spike is not for merging; section 4 lists what
it taught. **Since pull request 5 the server answers are reproducible from the repository:**
`tests/other-servers/compose.yaml` starts each server (images pinned by digest; its README says how), the check
on connect measures it (`craftcloud server check`, or `probe_a_real_server` with `CRAFTCLOUD_PROBE_URL`), and
the live suite runs against it with `CRAFTCLOUD_TEST_KIND` (section 13), asserting the mode it is expected to
get. The spike's own counts ("1 of 3 runs", the live-suite numbers of section 4) stand as measured that day; the
two plain WebDAV servers were measured with the check on connect on 2026-11-18.

## 1. Intent

People who already run Nextcloud, ownCloud or OpenCloud, or keep their files on a NAS, want the Craft apps' files
to sync through the storage they have, not through a second server. The sync engine is the part of CraftCloud that
is hardest to get right (it decides what to delete, on every device), and it has been hardened against OxiCloud
through two releases of testers' reports. Letting it talk to other servers is worth doing only if it stays exactly as safe.

So the work is less "add a WebDAV client" and more "say precisely what the engine relies on from a server, check
it on each server, and refuse what can't be met".

## 2. Goals, non-goals, anti-goals

### Goals

- The desktop client and the command line sync with:
  - **OxiCloud** (CraftCloud's own server), exactly as today;
  - **Nextcloud, ownCloud and OpenCloud**, using their own trash, versions and file ids where they have them;
  - **any other WebDAV server**, with CraftCloud's own trash and versions in `.craftcloud/` where the server has
    none.
- A **check on connect** (the probe) that measures what this server guarantees, and a plain answer to the person:
  two-way sync, download only, or not possible, and why.
- Sign-in without the client ever seeing the account password: the server's own browser sign-in, or an app
  password the person makes on the server.
- Every existing OxiCloud user sees no change at all.

### Non-goals

- The web frontend stays OxiCloud's. It is not ported to other servers.
- Server features beyond files: share links, users, admin, favourites, search, recent. On other servers the CLI
  says "not available on this server" for these.
- Several accounts or servers at once in one client (still one server per client).
- Files on demand (its own design, [FILES-ON-DEMAND.md](FILES-ON-DEMAND.md)) and delta sync.
- Nextcloud's bulk upload (`/remote.php/dav/bulk`) and chunked upload v2: they carry no preconditions, so the
  "no unguarded write" rule rules them out for good, not only for 0.3.0 (Q7).
- Requiring anything only an admin can switch on. Admin-only features (ownCloud's OAuth2 app, OpenCloud's basic
  auth) are a bonus where present, never a requirement.
- Upload-only ("backup") mode. It needs the same guarded creates and overwrites as two-way sync, so a server that
  can do it can do two-way sync anyway; it adds a mode without adding a server.

### Anti-goals: it fails, even if it works, when …

- **Any path can lose a file or silently overwrite one.** A server that ignores a write guard gets two-way sync
  anyway; a fallback quietly drops to a weaker check; a listing that failed reads as "empty"; a `DELETE` reaches a
  server whose `DELETE` is permanent for anything outside `.craftcloud/`.
- **A product name grants a capability.** "It says Nextcloud" must never stand in for "it honoured `If-Match`": the
  same product behind a different proxy, version or storage backend can behave differently. Profiles supply
  addresses; only the probe grants modes.
- **OxiCloud users notice.** A changed plan, a new prompt, a migrated keychain entry, a slower cycle.
- **The client asks for, or stores, an account password** on a server that can make app passwords.
- **Two rules for what the sync skips.** Per-server name rules must still go through the one function used by the
  local scan, the cloud listing and the saved state.
- **A refused mode is a dead end.** The person must be told what the server lacks, what they still get, and how to
  check again.

## 3. Constraints

| Constraint | Where it comes from | What it means here |
|---|---|---|
| The sync never loses a file | AGENTS.md, non-negotiables | Section 8, line by line, per capability |
| Never a weaker two-way sync | Decision for M9 | A server that can't guard writes gets download only, never two-way |
| `crates/core` builds for wasm | AGENTS.md, "Never break wasm" | Capabilities, modes, the probe's verdict and name rules are pure data and functions in `crates/core`; the trait, drivers, HTTP and the probe runner live in `crates/native` |
| The upstream server runs unmodified | AGENTS.md | Nothing changes on the OxiCloud side; other servers are used as they come |
| Upstream gaps go in UPSTREAM.md | AGENTS.md | OpenCloud's gaps are already reported upstream (reva#866, #959, #961); UPSTREAM.md links them and proposes nothing new to file |
| No panics, no `unsafe` | Rust rule 1 | All server answers are hostile input: hrefs outside the base, missing ETags, odd status codes |
| Shared libraries first | Core rule 16 | WebDAV requests and multistatus parsing go through `craft-dav` from `storytold/craft-libs`, as today; what it lacks (extra namespaces such as `oc:`, `nc:`, trash and version properties) is proposed there rather than parsed by hand here. Server profiles stay in CraftCloud |
| The client never asks for or stores the account password | AGENTS.md (amended in this change, Q11) | The server's browser sign-in where there is one; otherwise an app password the person makes on the server |
| Clean room | Rust rule 2 | Learn from public specifications (RFC 4918, RFC 8628, the servers' public API docs) and observed behaviour; never from another client's code |
| Licence allowlist | AGENTS.md | New crates only from the allowlist (`sha1` from RustCrypto is MIT OR Apache-2.0). The test servers (AGPL and Apache) are test containers, never shipped or linked |
| Rust only | Rust rule 4 | Nothing outside Rust is added |
| CI changes in their own pull request | Core rule 11 | The live jobs for other servers are a separate `👷 CI:` pull request |
| Time through a date crate | Rust rule 5 | Trash and probe stamps use `jiff`, as the rest of the client does |

## 4. What the spike taught

The spike (two commits on a local branch, not for merging) switched the WebDAV base and the auth header through
environment variables and short-circuited OxiCloud's REST calls. With that, the live suite (21 tests) passed on
OxiCloud 21/21, ownCloud 21/21, OpenCloud 21/21 and Nextcloud 20/21 (one timeout: about 4 uploads a second, so 301
files took over 60 s).

What had to change, and what the design does instead:

| The spike had to … | Spike hack (not to copy) | The design |
|---|---|---|
| Use another WebDAV base (`remote.php/dav/files/<user>/`) and strip it from hrefs | An environment variable | The driver holds its base URL; hrefs are parsed against it, absolute or relative, and an href outside it fails the listing (unknown, never "gone") |
| Send Basic auth | A `basic:<user>:<pass>` access token | A `Credential` type (section 10); never the account password |
| Trash without OxiCloud's REST ids | `DELETE` with `If-Match` | `RemoteStorage::trash`, which is a server-trash `DELETE` only where the probe saw it land in a trash, else a `MOVE` into `.craftcloud/trash/` |
| Restore from the trash | XML split by string search | Trash listing and restore per profile, parsed with `craft_dav` |
| Not snapshot versions | `snapshot` returns early | `keep_version` is a no-op only where the probe saw the server keep a version |
| Read free space | WebDAV quota props, OCS on Nextcloud | `free_space`, per profile (Nextcloud reports `-3` over WebDAV even with a quota set) |
| Create a file on OpenCloud | Upload under a temporary name, `MOVE` with `Overwrite: F`, then compare `oc:checksums` SHA1 | Not used: OpenCloud gets download only (Q1) |
| Find a folder's id | `oc:fileid` | Entries carry an optional file id; moves pair by it where present |

## 5. What the servers do (measured on 2026-11-16; reproducible since pull request 5, section 13)

The four servers of the design, as measured by hand on 2026-11-16, and the two plain WebDAV servers of the test
set (`tests/other-servers/compose.yaml`), as measured with the check on connect on 2026-11-18:

| Behaviour | OxiCloud 0.10.0 | Nextcloud 31 | ownCloud 10.15 | OpenCloud | `rclone serve webdav` 1.76.0 (as measured on 2026-11-18) | Apache httpd 2.4.71 `mod_dav` (as measured on 2026-11-18) |
|---|---|---|---|---|---|---|
| `PUT` with `If-None-Match: *` over an existing file | 412 | 412 | 412 | **ignored: overwrites** | **ignored: overwrites** (201) | 412 |
| `PUT` with a stale `If-Match` | 412 | 412 | 412 | 412, **not shown to be atomic** (reva#959) | **ignored: overwrites** (201) | 412 |
| `PUT`'s ETag equals `PROPFIND`'s | yes | yes | yes | yes | yes | **no ETag in the `PUT` answer** (the listing's is used) |
| ETag changes on every write (in a later second: Nextcloud and ownCloud reuse the tag within one second, so the checksum check of Q13 closes that window) | yes | yes in a later second; **reused within the same second** (Q13) | yes in a later second; **reused within the same second** (Q13) | yes | yes (from the modified time, also for the same bytes) | for other bytes; the same bytes keep their tag (`FileETag Digest`, below) |
| ETag survives `MOVE` | yes | yes | yes | yes | yes | yes |
| Folder ETags follow their contents | **not reliably** (UPSTREAM C8) | yes | yes | yes | folders have none | **no** |
| `oc:fileid`, stable across `MOVE` | no (404) | yes | yes | yes | no | no |
| `COPY` and `MOVE` | yes | yes | yes | yes | yes | yes |
| `MOVE`/`COPY` with `Overwrite: F` onto an existing name | 412 | 412 | 412 | 412, but **not atomic** under two creates at once (both won in 1 of 3 runs) | 412 | 412 |
| `MOVE` with a stale `If-Match` | (checked before, see `still_at`) | 412 | 412 | **ignored** (reva#961) | not measured | not measured |
| `DELETE` with a stale `If-Match` | (REST trash is used) | 412 | 412 | **ignored: deletes** | **ignored: deletes** (204) | 412 |
| Where WebDAV `DELETE` goes | gone for good | server trash | server trash | server trash | gone for good | gone for good |
| Restore from the trash | REST | `MOVE` to `trashbin/<user>/restore/<name>` | `MOVE` to the original path | `MOVE`, only through its `spaces/<space>/` URL | client trash (`.craftcloud/trash/`) | client trash |
| Versions | none (CraftCloud keeps them in `.craftcloud/versions/`) | per file id, follow renames (`/remote.php/dav/versions/<user>/versions/<fileid>/`) | per file id (`/remote.php/dav/meta/<fileid>/v/`) | per file id (`/remote.php/dav/meta/<fileid>/v/`) | none (client versions) | none (client versions) |
| `oc:checksums` | no | when the client sent one | yes | yes | no | no |
| Quota | REST `/api/auth/me` | OCS user endpoint (WebDAV says `-3`) | WebDAV quota props | WebDAV quota props | unknown | unknown |
| `PUT` into a missing folder | 409 | **404** | 409 | 409 | 409 | 409 |
| `Depth: infinity` | 403 | 207 | 412 | 400 | 207 | 403 |
| Backslash in a name | refused | 400 | 400 | 400 | not measured | not measured |
| Browser sign-in | OAuth device flow | Login Flow v2 | OAuth2 app (off by default) | OIDC | none (the WebDAV password, Q6) | none (the WebDAV password, Q6) |
| App passwords made by the person | no (device flow instead) | yes | yes | **yes** (9.1.1, measured 2026-11-18: `POST /auth-app/tokens` with the account's basic auth, or the web UI's "App tokens"; the token is the WebDAV password) | n/a (the WebDAV password, Q6) | n/a (the WebDAV password, Q6) |
| **Mode** | two-way (known, Q3) | two-way | two-way | **download only** (Q1) | **download only** (Q2) | **two-way**, with the client's trash and versions |

Apache's default ETag (`FileETag MTime Size`) is marked weak (`W/"…"`) for a second after a write, and a weak tag
never matches `If-Match` (RFC 7232's strong comparison): with it the check on connect refused Apache at step 6,
since overwriting the file it had just written with that tag answered 412. The test set's `httpd.conf` therefore
sets `FileETag Digest`, a strong tag from the content; with it every guard is honoured. A person's Apache with
the default ETags is refused, rightly: its listing can't be trusted to name the version a `PUT` just made.

## 6. Approaches considered

| Approach | For | Against | Verdict |
|---|---|---|---|
| **A. Branches inside the engine** (`if nextcloud { … }`), as the spike did | Smallest diff | Every safety rule gets one branch per server; the engine's tests no longer cover what runs; OxiCloud is touched by every change | Rejected |
| **B. One generic WebDAV client, no per-server knowledge** | One code path; the probe decides everything | Throws away what the known servers do better (trash, versions by id that follow renames, browser sign-in, quota); forces `.craftcloud/trash/` and `.craftcloud/versions/` into Nextcloud users' drives | Rejected as the only answer, kept as the fallback |
| **C. A storage layer that reports what it guarantees**, OxiCloud as the first driver, a generic WebDAV driver, and small profiles for known servers | The engine talks to one interface; the guards are in the types; OxiCloud moves behind it unchanged and is checked by today's tests; profiles add only addresses and features | A real refactor of `crates/native/src/sync.rs` first, before anything new works | **Chosen** |
| D. Use an existing sync library or rclone as the engine | Breadth of backends | Its safety rules aren't ours (it overwrites, deletes for good), licences, and it isn't Rust-in-process | Rejected |

## 7. The design

### 7.1 Where things live

```
crates/core (wasm-safe, no I/O)                 crates/native (desktop, CLI)
──────────────────────────────                  ──────────────────────────────
storage::Capabilities   what a server does      storage::RemoteStorage   the trait
storage::Mode           two-way / download      storage::oxicloud        driver 1 (today's code)
                        only / refused          storage::webdav          driver 2, generic
storage::mode_for()     Capabilities → Mode     storage::profile::{nextcloud, owncloud, opencloud}
storage::probe          the steps and the       storage::probe::run()    runs the steps over HTTP
                        verdict, as data        sync::Engine             uses only the trait
names::NameRules        per-server name rules
reconcile, restrict()   the plan, and the plan
                        cut to a mode
```

The decisions (what a capability allows, what a probe answer means, which plan a mode permits) are pure functions
in `crates/core`, so they get unit and property tests and build for wasm. Everything that sends a request is in
`crates/native`. A new crate is not needed: `crates/native` is already native-only.

### 7.2 The trait: no unguarded write

Every method that changes the server takes the precondition it must hold. There is no method that writes, moves
or deletes without one, so "forgot the guard" is a type error, not a review comment.

As built (pull request 1, `crates/native/src/storage.rs`):

```rust
pub trait RemoteStorage: Send {
    fn capabilities(&self) -> &Capabilities;

    /// A new sync pass: forget what the last one learnt (REST ids, an offline error).
    fn begin_pass(&mut self);
    /// The server didn't answer at all during this pass (taken).
    fn take_offline(&mut self) -> Option<String>;
    /// The sign-in is over: only a new sign-in helps.
    fn signed_out(&self) -> bool;

    /// One folder, one level. An error is "unknown": the caller holds that place, never takes it as empty.
    fn list(&mut self, folder: &str) -> Result<Vec<Entry>, StorageError>;
    /// Several folders, one level each (a sequential default; OxiCloud lists four at once).
    fn list_many(&mut self, folders: &[String]) -> Result<Vec<(String, Vec<Entry>)>, StorageError>;
    /// The ETag the listing gives a file (a write or download whose answer carried none).
    fn listed_etag(&mut self, path: &str) -> Result<String, StorageError>;
    /// Opens a download, from byte `from` on when given; the body follows with `Download::stream_to`.
    fn download(&mut self, path: &str, from: Option<u64>) -> Result<Download, StorageError>;

    /// Only if nothing is there. `Written::Conflict` when something is.
    fn create(&mut self, path: &str, file: &Path) -> Result<Written, StorageError>;
    /// Only if the server still has `if_match`. `Written::Conflict` otherwise.
    fn replace(&mut self, path: &str, if_match: &str, file: &Path) -> Result<Written, StorageError>;
    fn make_folder(&mut self, path: &str) -> Result<(), StorageError>;
    /// Never onto an existing name; a file only while it is still at its ETag.
    fn move_to(&mut self, from: &str, to: &str, guard: MoveGuard) -> Result<Moved, StorageError>;
    /// Into a trash, never for good. A file only if still at its ETag; a folder only if a fresh
    /// listing holds exactly the files given, or nothing at all.
    fn trash(&mut self, path: &str, guard: TrashGuard) -> Result<Trashed, StorageError>;
    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError>;
    fn restore(&mut self, item: &TrashItem) -> Result<(), StorageError>;

    /// Keep the version about to be replaced (no-op where the server keeps it itself).
    fn keep_version(&mut self, path: &str) -> Result<(), StorageError>;
    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError>;
    /// Free bytes; `needed` is the least a waiting upload needs.
    fn free_space(&mut self, needed: u64) -> Result<Option<u64>, StorageError>;

    /// Permanent removal, only inside CraftCloud's own `.craftcloud/` (probe files, old client
    /// trash, pruned versions). `OwnPath` can only be made for paths under `.craftcloud/`.
    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError>;
}
```

Where it differs from the first sketch, and why:

- **`download` returns a handle** (`Download`: `status()`, `header()`, `stream_to(out)`) instead of writing into an
  `out` it is given: the engine must see the status, `ETag` and `Content-Range` before it opens the part file (to
  append to it for a resume, or start over), and part files stay in the engine.
- **`free_space(needed)`** takes the smallest size a waiting upload needs: OxiCloud counts storage use late, so its
  driver asks an admin's server to count again when the free space is below that (at most once a minute).
- **`keep_version(path)`** has no `current` ETag: the OxiCloud driver reads the current version itself (to skip
  keeping the same bytes twice), as it always did, so a passed-in ETag would go unused.
- **`list_many`**: a cycle lists every folder every cycle; OxiCloud's driver keeps listing four at once, so the
  refactor doesn't make a cycle slower (a sequential default serves other drivers).
- **`listed_etag`**: a write or a download whose answer carries no `ETag` reads it from the parent's listing; one
  provided method serves the engine and the drivers.
- **`begin_pass`**: what a driver learns during a pass (OxiCloud's REST ids) is forgotten at the start of the next.
- **`take_offline` and `signed_out`**: the engine tells "offline" from "signed out" from the connection's state when
  a pass ends, not from the error that stopped it, so a failure it shrugs off (the free-space lookup) still counts,
  as before. `StorageError` (`Status`, `NotFound`, `NoRoom`, `Refused`, `Failed`) carries the rest, in the engine's
  wording.
- An ETag is a plain `&str`; `Entry`, `Written`, `Moved`, `Trashed`, the guards and `StorageError` are in
  `crates/native::storage`.

The types:

- `Entry` is today's `dav::Entry` plus an optional `file_id` and an optional `checksum`.
- `Written` is `Saved { etag }` or `Conflict` (a `file_id` comes with the first driver that has ids). A 412 is a
  `Conflict`; the engine makes a conflict copy, as today.
- `StorageError` keeps today's distinctions: no room (507), refused for what it is (4xx, waits until the file
  changes), a status the call can't take, not found, and other; offline and signed out come from `take_offline` and
  `signed_out` (above).
- The engine's existing rails (mass-delete hold, unreadable places held, the two-second settle, part files and
  resume) stay in the engine and don't move into drivers.

### 7.3 Capabilities and modes

`Capabilities` is what the probe measured, filled in by the profile where the probe can't measure it (addresses,
how sign-in works):

| Field | Values | Measured by |
|---|---|---|
| `create_if_absent` | honoured · ignored | probe |
| `replace_if_match` | honoured · ignored | probe |
| `delete_if_match` | honoured · ignored · n/a (REST trash) | probe |
| `move_no_overwrite` | yes · no | probe |
| `etag_changes_on_write`, `etag_kept_by_move`, `put_etag_is_listed` | yes · no | probe |
| `folder_etags_follow` | yes · no · not trusted | probe (OxiCloud: not trusted, by profile) |
| `missing_parent` | 404 · 409 · other | probe |
| `depth_infinity` | yes · no | probe |
| `copy` | yes · no | probe |
| `file_ids`, `checksums` | none · present (and which hash) | probe |
| `case_insensitive` | yes · no | probe |
| `trash` | server (restore by profile) · client (`.craftcloud/trash/`) | probe confirms a server trash with a round trip |
| `versions` | server (by file id) · client (`.craftcloud/versions/`) · none | probe confirms server versions by replacing a file |
| `quota` | OxiCloud REST · OCS · WebDAV props · unknown | profile, then one request |
| `names` | `NameRules` | profile, extended by the probe's name checks |

`mode_for(&Capabilities) -> Mode`, a pure function in `crates/core`:

| Mode | Needs | What the person gets |
|---|---|---|
| **Two-way** | `create_if_absent` honoured · `replace_if_match` honoured · `move_no_overwrite` · `etag_changes_on_write` · a trash (server, or client through `MOVE`) · versions (server, or client through `COPY`) | Today's sync |
| **Download only** | Listings with ETags and `GET` | Cloud → this computer. Nothing is written to the server. Local changes stay local (section 9) |
| **Refused** | — | Listings fail, files come without ETags, an ETag that doesn't change when the bytes do, or a probe that didn't finish |

A probe that errors out or times out is **refused**, never "generic, probably fine". `delete_if_match` ignored does
not lower the mode (section 8, "Delete only if unchanged").

### 7.4 Profiles

A profile is small: it knows where things are, never what they guarantee.

| | OxiCloud | Nextcloud | ownCloud | OpenCloud | Other WebDAV |
|---|---|---|---|---|---|
| Detected by | `GET /api/auth/status` | `GET /status.php` (`productname`) | `GET /status.php` (`productname` "ownCloud") | its well-known config | the person chooses it |
| WebDAV base | `/webdav/` | `/remote.php/dav/files/<user id>/` | `/remote.php/dav/files/<user id>/` | to verify (personal space URL) | the URL the person gives |
| User id | from the token | `current-user-principal` (the login name can differ) | `current-user-principal` | to verify | from the URL |
| Trash | REST (today) | WebDAV trashbin (`trashbin/<user id>/trash/`, restore to `restore/<name>`) | WebDAV trash bin (`trash-bin/<user id>/`, items by file id, restore by `MOVE` to the original path) | spaces trash | client |
| Versions | client | server, by file id (`versions/<user id>/versions/<file id>/`) | server, by file id (`meta/<file id>/v/`; a restore reads the version and writes it with a guarded `PUT`) | server, by file id | client |
| Quota | REST | OCS user endpoint | WebDAV props | WebDAV props | WebDAV props, else unknown |
| Sign-in | device flow | Login Flow v2 | app password | OIDC (to verify) | app password |

The OxiCloud driver is today's code moved behind the trait. Nextcloud, ownCloud and OpenCloud use the
generic WebDAV driver with their profile.

### 7.5 The probe

**Where.** In `.craftcloud/probe/<device>-<random>/` on the server, so two devices probing at once never meet. It
starts by sweeping probe folders a crashed probe left. A folder's age is read **only from the server's clock**:
its `getlastmodified` against the `Date` header of the same listing, never against this computer's clock, so a
device whose clock is off can't take another device's running probe for an old one. Only folders older than a
day by that measure go (a probe takes seconds), and only through `purge_own`, inside `.craftcloud/`. A listing
without a `Date` header or a time sweeps nothing.

**When.** On connect; at start when the cached result is older than a day or the server's version string changed;
and on "Check again". The result is cached in the settings folder (`storage-check.json`: the date, the server's
version string, each check's answer) and shown in Settings and by `craftcloud server check`. Not on OxiCloud by
default (Q3).

**What it does, in order** (each step a pure request/expectation pair in `crates/core::storage::probe`, run by
`crates/native`):

| # | Step | Expect | If not |
|---|---|---|---|
| 1 | `PROPFIND` depth 0 of the base, then depth 1 of `.craftcloud/` (made if missing) | 207, entries with ETags | Refused |
| 2 | `MKCOL` the probe folder | 201 | Refused |
| 3 | `PUT a.txt` with `If-None-Match: *` | 201, an ETag (else one more `PROPFIND`: `put_etag_is_listed` no) | Refused |
| 4 | `PUT a.txt` again with `If-None-Match: *`, other bytes | 412, bytes unchanged | `create_if_absent` ignored: no two-way |
| 5 | `PUT a.txt` with a made-up `If-Match` | 412, bytes unchanged | `replace_if_match` ignored: no two-way |
| 6 | `PUT a.txt` with the right `If-Match`, other bytes | 2xx, a new ETag | ETag doesn't change: Refused |
| 7 | `PUT a.txt` with the same bytes again | note whether the ETag changes | Only noted |
| 8 | `PUT sub/deep/b.txt` (parents missing) | 404 or 409, nothing made | Recorded as `missing_parent` |
| 9 | `MKCOL sub`, `sub/deep`, `PUT` there; compare the probe folder's ETag before and after | changed | `folder_etags_follow` no |
| 10 | `PROPFIND` with `oc:fileid`, `oc:checksums` | present or not | Recorded |
| 11 | `MOVE a.txt` → `c.txt` with `Overwrite: F` | 201; same ETag and file id after | No move pairing by ETag/id |
| 12 | `MOVE b.txt` onto `c.txt` with `Overwrite: F` | 412, both unchanged | `move_no_overwrite` no: no two-way |
| 13 | `COPY c.txt` → `d.txt` with `Overwrite: F` | 201 | `copy` no |
| 14 | `PUT C.TXT` next to `c.txt` and list | two entries | `case_insensitive` yes |
| 15 | `DELETE c.txt` with a made-up `If-Match` (in the probe folder, so even a permanent `DELETE` loses only probe files) | 412 | `delete_if_match` ignored |
| 16 | Known profiles: `DELETE d.txt`, find it in the trash listing, restore it | found, back with its bytes | No server trash: client trash |
| 17 | Known profiles: replace `c.txt` once more, list its versions by file id | one more version | No server versions: client versions |
| 18 | `PUT` a few hard names (trailing space and dot, `:`, a 255-byte name, an NFD name) | 2xx or 4xx | Recorded in `NameRules` |
| 19 | `Depth: infinity` on the probe folder | 207 or not | Recorded |
| 20 | Clean up: delete the probe folder | gone | Swept next time |

On known servers step 20 leaves one probe folder in the person's server
trash; that is the price of checking the trash for real (Q8).

Two things the probe **can't** measure, and how the design treats them:

- **Atomicity.** OpenCloud's `MOVE` with `Overwrite: F` lost a race in 1 of 3 runs. A probe can't race itself
  reliably, so a profile may mark a server's guard as not atomic from measurements like these, which only ever
  lowers the mode, never raises it.
- **Intermittent faults.** OxiCloud's folder ETags didn't always change. A probe that passes once proves little, so
  skipping unchanged folders stays off everywhere in 0.3.0 (Q4).

The engine also watches at runtime: a create that the cloud listing shows replaced someone else's file, or a
replace that succeeds on a version the listing says is newer, stops two-way sync for the session, re-runs the
probe, and says so in the activity list.

As built (pull request 3, `crates/core/src/storage/probe.rs` and `crates/native/src/storage/probe.rs`):

- **The probe is a state machine, not a fixed list.** `Probe::next()` gives the request to send now (a `Request`:
  the step, the method, a cloud path under the WebDAV base, the guard headers, a body, a destination for `MOVE`
  and `COPY`), `Probe::observe(&response)` moves on, `Probe::verdict()` gives the `Mode`, the `Capabilities` and
  every step in plain words (`StepResult`). The twenty steps of the table are the steps; several take more than
  one request (step 1 lists the base and `.craftcloud/probe/` and makes them when missing; steps 4 and 5 follow
  the refused write with a `GET` to see the bytes stayed; step 3 always lists the new file, so `put_etag_is_listed`
  compares the two tags). A `PUT` is built only through a guarded constructor (`If-None-Match: *` or `If-Match`),
  so an unguarded write can't be expressed. Hrefs are read against the base's path (`Base::relative`); one outside
  it fails the listing. The runner checks every request's guard once more before sending (`guarded`): a `PUT`
  without its guard, a `MOVE` or `COPY` without `Overwrite: F` or outside `.craftcloud/probe/`, or a `DELETE`
  anywhere but inside `.craftcloud/probe/` (an `OwnPath`) ends the probe, refused.
- **A second between writes to the same file.** Measured while building this: Nextcloud 31 and ownCloud 10.15
  keep a file's ETag when it is overwritten within the same second as the write before it (the tag is reused
  while the file's modified time, in whole seconds, is unchanged; 3 of 3 rounds on each, with `curl`). The
  probe's first run against ownCloud was therefore refused at step 6. The two overwrites of the test file
  (steps 6 and 7) are marked `settles`, and the runner waits `Limits::settle` (1.1 s) before each, so the
  probe measures what § 5 measured. This narrows § 5's "ETag changes on every write" to "on every write in a
  later second"; what it means for the engine (two devices writing one file within a second) is for the
  maintainer to weigh, not changed here.
- **Refused wins.** A transport error, a request over the per-step limit (30 s), a run over the total limit
  (180 s), a `verdict()` asked for before the end, a version tag that doesn't change when the bytes do, a file
  listed without one, a failed listing, or a server that won't make the probe folder or create a file: all
  `Mode::Refused`, with the reason in words. When the server still answers, the probe goes straight to step 20
  and deletes its folder; when it doesn't, the folder stays for the next probe to sweep.
- **The sweep** is a pure function on seconds (`stale_probe_folders`): each folder's `getlastmodified` against the
  `Date` header of the same listing, both parsed by `server_seconds` (jiff's RFC 2822 parser, which core's `std`
  feature already has), older than a day goes. No `Date`, or a folder without a time, sweeps nothing. The runner
  never looks at this computer's clock for it.
- **The transport** is a two-method trait in `native` (`Transport::call` and `upload`, the shapes `Session` has;
  `Session` implements it), so the generic WebDAV driver of pull request 4 runs the same probe with an app
  password. The probe's bodies are bytes, so it uses `call` only.
- **Steps 16 and 17** take a profile's check through `ExtraCheck` (`next(&Learnt)`, `observe`, `confirmed`), handed
  in as `Profile { trash, versions }`. No profile exists yet, so the verdict records `Trash::Client` and
  `Versions::Client` (or `Versions::None` without `COPY`, Q2) exactly as § 7.3 says, and the step says why.
- **Step 18** sends the hard names (a trailing space, a trailing dot, a colon, a 255-byte name, an NFD name) and
  records each answer in words. `NameRules` is **not** widened by what it sees: the rules are the skip rule every
  device must agree on, and a server that accepts a trailing space today proves little about the next one; the
  capabilities keep the strict rules (a backslash, a control character, more than 255 bytes). Widening them per
  server is a later decision, with its "upgrade all devices together" entry.
- **The saved check** (`StoredCheck`, `storage-check.json` in the settings folder, written whole or not at all)
  holds the date, the server's version string, the base, `measured`, the mode, the capabilities, the capability
  table in words and every step. Every field has a default on the safe side (`Mode::Refused`, no capabilities, the
  strict names), so an older or newer client reads it without taking it for more than it says.
- **The wording** of the capability table is one function, `probe::checks(&Capabilities)`, which `craftcloud server
  check` prints for OxiCloud's known capabilities today and the desktop's "What was checked" (pull request 10)
  reuses.
- `Capabilities`, its enums, `Mode` and `NameRules` serialize (snake case; `two_way`, `download_only`, `refused`),
  and `Capabilities::unknown()` is the server nobody knows: every guard ignored, nothing trusted.

As built (pull request 4a, the first of two under § 14's row 4; `crates/native/src/storage/webdav.rs`,
`crates/native/src/experimental.rs`, `secrets.rs`, `session.rs`, `apps/craftcloud/src/cli/webdav.rs`):

- **The opt-in** is `craftcloud_native::experimental::experimental_webdav()`, true only when
  `CRAFTCLOUD_EXPERIMENTAL_WEBDAV` is exactly `1`; `experimental::allowed(settings, experimental)` is the one decision
  (pure, so it is tested without the process's environment), asked by the command line before every command but
  `logout` and `apps`, by the daemon and by the desktop client. Without it a settings file naming another kind is
  refused with the message of § 16 (exit code 1, kind `error`; the desktop's window opens on the Connect sheet with
  it); `--kind` and `--user` are hidden from `--help` and `--kind webdav` is a usage error. With it, `nextcloud`,
  `owncloud` and `opencloud` in the settings are refused too ("can't sync with this kind of server yet"): a known
  server is never taken for a plain WebDAV server, whose client trash and versions don't belong in its drive.
- **`Credential`** (`secrets.rs`) is `Bearer(Tokens)` or `AppPassword { user, secret }`. A `Bearer` entry is written
  exactly as 0.2.0 wrote it (no `kind` field; a test compares the bytes), an app password with `kind:
  app_password`; an entry with a `kind` this build doesn't know isn't a sign-in. `TokenStore::load` (tokens) answers
  nothing for an app password, so no OxiCloud path ever takes one for tokens. `AppPasswordSession` (`session.rs`)
  sends Basic auth, refreshes nothing, takes a 401 as signed out, and refuses at construction to send a password over
  plain http to anything but this computer (`127.0.0.1`, `::1`, `localhost`). Its `Debug` prints no secret.
- **The generic driver** does what § 8's "Server lacks it" column says: listings read against the base
  (`probe::parse_listing`, which now carries `getcontentlength` and `quota-available-bytes` too), a create with
  `If-None-Match: *`, an overwrite with `If-Match`, `still_at` and `cloud_holds` before a move or a trash as the
  OxiCloud driver does, a trash by `MOVE` with `Overwrite: F` into `.craftcloud/trash/<time>-<device>/<path>` (the
  entry's folder is made first, with the item's parents in it; a 412 on the `MOVE` is an error tried again next pass,
  never a forced move), a folder as one `MOVE`, versions by `COPY` into `.craftcloud/versions/<path>/<time>` with the
  same code OxiCloud's driver uses (`storage::versions`, over the probe's `Transport`), free space from
  `quota-available-bytes` (negative or absent: unknown), and a `DELETE` only through `purge_own`.
  `trash_list` reads the entries' folders: one item per entry, found by walking down while a folder holds exactly
  one folder (a file is the item, and so is a folder holding nothing or several things); a trashed folder whose only
  content is one folder lists as that folder, and a restore puts the same tree back. `restore` is a `MOVE` back with
  `Overwrite: F`, `names::restored_name` when the place is taken, and purges the entry's folder once it holds
  nothing. `WebDav::connect` reads `storage-check.json` when it was saved for this base, else runs the probe and
  saves it; a refused check leaves `Capabilities::unknown()`, so the engine refuses to sync and says why.
  `already_kept` compares content hashes the way OxiCloud's ETags carry them; another server's ETags carry none, so
  a retried overwrite there may keep the same bytes twice (harmless; pruned with the rest).
- **The engine** builds the driver on its own thread (`sync::start_webdav` takes a connect closure, since the
  check on connect may take a while); `status` shows "checking the server" meanwhile. `SyncProgress.mode` and
  `status --json`'s `mode` say what the server allows. Under download only the uploads the cut takes out are said
  once per version of the file ("changed on this computer, not uploaded: this server only syncs downwards"; "new on
  this computer, …") and listed as `not_synced` with the reason `not_uploaded`. On a case-insensitive server two
  local names that differ only in case get the clash rule the other way round: the one the cloud has keeps
  syncing, the other stays here and is said once; this computer's own name rule is unchanged by it.
- **The command line**: `login <WebDAV address> --kind webdav --user NAME` (the password from `CRAFTCLOUD_APP_PASSWORD`
  or a prompt, never an argument) checks the address with one listing, saves the sign-in under the server's origin,
  sets `storage` and `dav_url`, runs the check on connect and prints the mode; `server check` on such a server runs
  the probe afresh and prints every step; `ls`, `get`, `put`, `save`, `mkdir`, `mv`, `rm`, `files` and `sync` go
  through the trait; `share`, `star`/`starred`, `recent`, `search`, `usage`, `cp` and `server info|users|settings`
  answer "not available on this server" with exit code 10 (kind `unsupported`), and so do `trash` and `versions`
  until 4b wires them through the trait. OxiCloud's commands keep their REST paths unchanged. A sync folder holding
  another sync client's database (`.sync_*.db`, `.owncloudsync.log`, `.csync_journal.db`) is refused by the folder
  claim, for every kind of server (exit code 5).
- **The desktop client** syncs CraftCloud's own server only until pull request 10: with the opt-in and a settings
  file naming another kind, its window opens on the Connect sheet and says to use `craftcloud daemon` or `sync`.
  Under download only its header, tray and headline say "Download only".
- **Done in pull request 4b**, as § 14 splits the row: see the as-built note below.
- **Carried from pull request 3, not decided here:** Nextcloud 31 and ownCloud 10.15 keep a file's version tag when
  it is overwritten within the same second as the write before it. The generic driver changes nothing about it;
  the engine's `If-Match` can't tell two writes to one file apart within one second on those servers.

As built (pull request 4b, the second of two under § 14's row 4; `crates/native/src/storage/webdav.rs`,
`crates/native/src/sync.rs`, `crates/core/src/storage/probe.rs`, `apps/craftcloud/src/cli/webdav.rs`):

- **The client trash's retention** (§ 16, Q5). `RemoteStorage::sweep_trash` (a no-op where the server keeps its own
  trash) removes entries of `.craftcloud/trash/` older than 30 days (`dav::TRASH_RETAIN_SECS`) **by the server's
  clock**: each entry folder's `getlastmodified` against the `Date` header of the same listing, through the shared
  pure `core::storage::probe::older_than` that `stale_probe_folders` now also uses, never this computer's clock. A
  listing without a `Date`, or an entry without a time, removes nothing. Removal is `purge_own` only, inside
  `.craftcloud/`. The engine calls it after a clean cycle, at most once a day per client (a monotonic-clock rate
  limit, not a deletion decision). Never sooner for quota reasons.
- **The trash and version commands through the trait.** On another WebDAV server the command line routes
  `trash ls|restore|rm|empty` and `versions`/`versions … restore` through the storage trait (`cli/webdav.rs`): `ls`
  shows each item's original cloud path (from the entry's name) and when it was trashed, reading only; `restore`,
  `rm`, `empty` and a version restore write, so they need two-way sync (`webdav_for_writes`). `restore` is a `MOVE`
  back with `Overwrite: F`, `names::restored_name` when the place is taken (`--as PATH` restores elsewhere); `empty`
  and `rm` purge inside `.craftcloud/trash/` only; `versions`/`restore` read and restore `.craftcloud/versions/`
  through the same code OxiCloud uses (`storage::versions`, now with a `restore` that snapshots the current file
  first, so the restore can be undone).
- **The runtime watch** (§ 7.5, engine). A file this client just wrote that comes back from the next listing as
  someone else's — the ETag the write answered isn't the one listed **and** the listed size isn't the one uploaded —
  stops two-way sync for the rest of the run (`Engine::effective_mode` forces download only), re-runs the check
  (`RemoteStorage::recheck`, which saves it and takes the fresh capabilities), and says so once in the activity list
  and in `status`. It adds no request on a well-behaved server (the next listing shows exactly what the write
  answered), so OxiCloud's golden logs are unchanged. Two `fake::Flags` stage it (`create_keeps_other`,
  `replace_keeps_other`).
- **The check's re-run schedule** (§ 7.5, "When"). `WebDav::connect` reuses the saved check only while it is less
  than a day old (`checked_at`, this computer's clock — cache freshness, not a server-clock safety decision) and the
  server's version string is unchanged; otherwise it re-probes and saves. The version string is a plain WebDAV
  server's `Server` header (none, or the richer `status.php`, come with the profiles of PRs 6–8); an unknown version
  never forces a re-check on its own, so a momentary blip keeps a good saved check. `server check` still re-runs the
  probe. The mode in use always comes from the saved check, never from a product name.
- **Left as is (the maintainer decides):** 4a's command line refuses CLI writes (exit code 10) on a download-only
  server (`webdav_for_writes`), beyond what § 11 lists; a trashed item's restored path isn't echoed back when a
  clash renamed it (the generic `restore` reports the original path).

As built (pull request 5, § 14's row 5; `crates/native/tests/live.rs`, `crates/native/tests/live/kind.rs`,
`apps/craftcloud/tests/cli_live.rs`, `tests/other-servers/`):

- **One suite, the cloud side per kind.** `CRAFTCLOUD_TEST_KIND` picks the server (`oxicloud` by default: the
  suite CI's `live` job runs, its REST helpers and its 60 s deadline unchanged). The tests themselves changed
  only in how they reach the cloud: a `Live` corner holds a `Cloud` that is OxiCloud over REST or, for every
  other kind, the generic WebDAV driver (the trait: `list`, `trash_list`, `restore`, `trash` with its guard for
  "another client deleted it", `keep_version`, `list_versions`, `capabilities`) plus the test's own raw WebDAV
  client for what another client or the person does (an unguarded `PUT`, a `DELETE`) and for the cleanup. So
  when a server's profile lands (rows 6 to 8), the same tests check its trash and versions through the same
  calls. Until then every other kind runs the generic driver: on Nextcloud and ownCloud that means the client's
  trash and versions in their drive, which the profiles replace.
- **The mode is asserted, never assumed.** `Server::from_env` fixes the mode the server must get (§ 5 and § 16
  per kind; `CRAFTCLOUD_TEST_MODE` for a plain WebDAV server, where it depends on the server); the connect
  closure returns an error when the check on connect gives another mode, and every `sync` pass asserts
  `SyncProgress.mode`. The two-way tests run where two-way is expected and say why they don't otherwise; the
  download-only suite (§ 9) the other way round; `the_server_gets_the_mode_expected` always runs. A kind the run
  can't do (an unknown word, or another kind without `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`) fails at once in every
  test, never passes without running; `the_kind_is_checked_before_anything_runs` checks that decision in the
  gate, without a server.
- **Sign-in as a person would.** Nextcloud: `GET /ocs/v2.php/core/getapppassword` with the account's basic auth
  once; ownCloud 10 has no API for it, so the suite goes through the web as a browser does (`curl`: the login
  page's request token, the sign-in form, Settings › Security's `authtokens` call); plain WebDAV: the WebDAV
  password (Q6); OpenCloud: `POST /auth-app/tokens` with the account's basic auth once (through the test
  server's `PROXY_ENABLE_BASIC_AUTH`), the token then being the WebDAV password, which settles § 5's "to
  verify": OpenCloud 9.1.1 makes app tokens for a person, on by default. The app password is made once per run
  and never printed; `CRAFTCLOUD_TEST_APP_PASSWORD` hands one in.
- **The download-only suite** (§ 9, against OpenCloud and `rclone serve webdav`): a cycle writes nothing (the
  whole server tree, path by path and ETag by ETag, before and after three passes with a local change, a new
  local file and a local delete); the change and the new file stay here, are said once per version and listed as
  `not_uploaded`; a cloud change over a local change makes a local conflict copy and nothing goes up; a file
  deleted in the cloud lands in the local `.craftcloud/trash/<time>-<device>/<path>` with its bytes, a folder of
  12 only after the mass-delete hold is confirmed, and a file changed here meanwhile stays; a failed listing (a
  small proxy in the test answers 500 to `PROPFIND` under one folder) ends the pass without a result and nothing
  under the folder is taken as deleted, until it lists again.
- **Per-server deadlines.** Nextcloud takes about 4 uploads a second (Q7), so a pass may take 300 s there (the
  300-file test keeps its files: its check that the activity list is full needs more than 200 uploads); 180 s on
  ownCloud, OpenCloud and plain WebDAV; OxiCloud's 60 s as before.
- **What stays OxiCloud's:** the two quota tests (its admin API makes the users), the star in the renamed-folder
  test and the UI transport's test; the renamed-folder and discarded-deletes tests check the folder's id only where
  the server has ids (`oc:fileid`). The T-019 check of a version's content time needs OxiCloud's tags.
- **Cleanup per kind.** OxiCloud as before (REST trash, then purged). Elsewhere the test's top-level folders, its
  `.craftcloud/versions/<folder>` and the client-trash entries holding only its things are deleted; on Nextcloud
  and ownCloud they are taken out of the server's trash bin too (`<name>.d<seconds>`), so a run leaves nothing
  behind but the probe folders in the trash (Q8).
- **Measured on the plain servers** (§ 5): `rclone serve webdav` 1.76.0 gets download only, as Q2 says; Apache
  httpd 2.4.71 with `mod_dav` keeps every guard and gets two-way with the client's trash and versions, once its
  ETags are content digests (its default time-based tags are weak for a second after a write and failed step 6).
  The suite's `webdav` kind runs against both.
- **Found by the suite, left for its own pull request:** on Apache two live tests (a trashed folder restored;
  discarded cloud deletes put back) and the command line's `trash rm` of a folder end "signed out". Apache
  answers 301 to a `PROPFIND` or `DELETE` of a folder named without its trailing slash (mod_dir's
  `DirectorySlash`, on by default); the HTTP layer follows the redirect as a `GET` without the `Authorization`
  header, and `AppPasswordSession` reads the 401 as a revoked password. The generic driver names a folder
  without the slash in `restore` (`is_folder`) and `purge_own`. Two fixes to weigh: folder URLs always with the
  slash in the driver, and no redirects followed for WebDAV methods (a redirect answered to a `PROPFIND` is
  not a listing). The test set keeps Apache as a person's would be, so those three fail there until then.
- **No CI job** (core rule 11; § 14 row 9).

As built (pull request 6, § 14's row 6; `crates/core/src/api/nextcloud.rs`, `crates/native/src/storage/profile.rs`
and `profile/nextcloud.rs`, `crates/native/src/storage/{webdav,probe}.rs`, `apps/craftcloud/src/cli/{account,check,remote,webdav}.rs`):

- **The profile hook.** `storage::profile::ServerProfile` is what a known kind of server adds to the generic driver,
  every method with a default that adds nothing: the server's version string (`status.php`), the probe's
  known-server checks of steps 16 and 17, "verify writes by checksum" (`Option<Checksum>`, Q13), the server's own
  trash (`trash_list`, `restore`), its versions (`list_versions`, `version_download`) and its quota endpoint.
  `profile::for_kind(Storage, &Base)` gives the kind's profile (`Generic` for a server nobody knows);
  `WebDav::connect_with` and `WebDav::with_profile` take it. The driver asks the profile for the trash only where the
  check confirmed `Trash::Server` (a `DELETE` then lands in the server's trash: a file's with `If-Match`, a folder's
  after `cloud_holds`), and for the versions only where it confirmed `Versions::Server` (`keep_version` and
  `move_versions` are then no-ops; the server keeps them by file id). The ownCloud and OpenCloud profiles (rows 7
  and 8) plug into the same hook.
- **The probe on another DAV root.** A known-server check's `Request` may name a `root` (the trash bin, the
  versions) and, for a restore, `restores` (the cloud path the server said the item came from). The runner's
  `guarded` takes the probe's folder and allows on such a root only a listing, a `GET`, and a `MOVE` from
  `trash/<name>` to `restore/<name>` of an item whose origin is inside the probe's own folder (no `Overwrite: F`
  there: the restore address is the server's endpoint, not a name in the person's tree, and Nextcloud 31 answers
  412 to the header, measured);
  a `DELETE` there (purging the person's trash, Q8), any other write, or a `MOVE` anywhere else is refused
  (measured: Nextcloud 31 does move a trashed item to any destination under the files base, so the guard matters).
  `Learnt` carries `d_exists` and `c_bytes` for the checks.
- **Nextcloud's profile** knows: the files base `/remote.php/dav/files/<user id>/` (the user id from
  `current-user-principal`; the login name can differ); the trash bin and its restore `MOVE`; the versions by file
  id (the listing names the current file among them, which `list_versions` leaves out by its modified time; a
  restore is a guarded write, below); the OCS
  user endpoint's `quota.free` (`-3`: no limit); `status.php` for the version string; `missing_parent` 404
  (recorded by the probe as before). Step 16 deletes the probe's `d.txt`, finds it in the trash bin by its
  original location, restores it and reads it back; step 17 overwrites `c.txt` once more (in a later second) and
  needs an earlier version besides the current in the listing. Without a profile, or on a server whose trash bin
  doesn't answer, both stay `Client`. The answers are recorded in `crates/core/tests/fixtures/nextcloud-31/`
  (status, Login Flow start and pending poll, OCS user, principal, a folder listing with checksums, a file
  written without one, versions, the trash bin; the finished poll is the public documentation's shape).
- **A version restore is a guarded write, on every server.** A profile only names where a version's bytes are
  read (`version_download`, a `GET`); the driver (`WebDav::restore_server_version`) reads the file's version tag,
  reads the version, and writes it back with its guarded `replace` (`PUT` with `If-Match`, plus the checksum check
  below). A change landing in between answers 412: nothing is restored, the newer file stays
  (`Restored::Changed`; the command line exits with code 5, as it does for CraftCloud's own server). Nextcloud's
  own restore (`MOVE` of the version to `…/versions/<user id>/restore/target`) is never sent: it names no version
  of the file it replaces, so it is the unguarded write § 7.2 excludes, and since an overwrite within one second
  leaves no version there (UPSTREAM.md E1) the bytes it replaced could be gone for good. The server keeps the
  bytes the `PUT` replaces as a version itself. The ownCloud and OpenCloud profiles get the same path.
- **The checksum check (Q13).** On a profile that asks for it, every `PUT` carries `OC-Checksum: SHA1:<hex>` (the
  `sha1` crate, RustCrypto, MIT OR Apache-2.0), and after a 2xx a depth-0 listing must show that checksum among
  `oc:checksums`; anything else is `Written::Conflict`, so the engine makes a conflict copy of the local bytes and
  downloads the server's, and the activity list says "conflict … yours kept as …". Nextcloud stores the checksum it
  was sent without checking it, lists it only for a file whose last `PUT` carried one, and sends none in the `PUT`'s
  answer, so the read-back is one more request per write. The scripted server (`fake::Flags`) has
  `etag_reused_within_second`, `checksums_from_client_only` and `other_writes_after_put` for it; the driver test and
  the engine test show the write landing silently without the check and as a conflict copy with it.
- **Sign-in.** `craftcloud login <server> --kind nextcloud` runs Login Flow v2 (`profile::nextcloud::sign_in`): the
  browser opens the server's page, the client polls (404 until done, every second, twenty minutes at most) and
  stores the login name and the app password made for this device, listed on the server under the client's
  `User-Agent` ("CraftCloud on <host>"). With `--user NAME` an app password made in Settings › Security is taken
  instead (from `CRAFTCLOUD_APP_PASSWORD` or a prompt), which is what the live suites use. Without `--kind`, and
  with the opt-in, a server whose `status.php` says Nextcloud is signed in to as one (detection suggests the kind;
  the check on connect still measures everything). `logout` deletes the app password on the server
  (`DELETE /ocs/v2.php/core/apppassword`) and then forgets it. The desktop client still refuses every other kind of
  server until row 10.
- **The command line** on Nextcloud: the same commands as on another WebDAV server, through the trait; `trash ls`
  and `trash restore` show and restore the server's own trash, `versions` and `versions … restore` the server's own
  versions; `trash rm` and `trash empty` answer "not available on this server" (code 10): purging a person's trash is
  a permanent delete outside `.craftcloud/`, which CraftCloud never sends (Q8). `server check` says the kind and the
  server's version, and runs the profile's steps 16 and 17.
- **The scripted server** answers like Nextcloud with `Flags::nextcloud` (`status.php`, OCS, the Login Flow, the
  principal, a trash bin that every `DELETE` lands in, versions kept on every overwrite, 404 for a missing parent),
  so the profile, the probe's round trips, the driver and the engine are tested without a server; the live suite
  (`CRAFTCLOUD_TEST_KIND=nextcloud`) connects with the profile and asserts the server's trash and versions:
  26 of 26 against Nextcloud 31.0.14 from `tests/other-servers/compose.yaml` on 2026-11-18, in 242 s with the
  server's trash and versions granted (and 223 s in an earlier run that still got the client's), the two tests
  pull request 5 saw fail now and then (the renamed folder, the same bytes written again) among them both times;
  the command line's live test 3 of 3.
- **Measured while building this, and recorded in UPSTREAM.md E1:** Nextcloud names a version by its second, so an
  overwrite within the same second as the write before it leaves no version of the earlier bytes (whenever two
  writes shared a second, the ETag stayed and the versions listing had one entry). The sync never writes a file
  twice within a second (a cycle lists first); `craftcloud put --force` right after `put` would, and the version it
  loses is the bytes just uploaded. The live command-line test spaces those two writes on Nextcloud.
- **Left as is:** pairing moves by `oc:fileid` in `reconcile` (Nextcloud keeps a file's ETag across a `MOVE`, § 5,
  so renames pair by ETag as before, and the live test checks the folder's id stays); the desktop's sign-in (row
  10); the client-trash entries a run of the generic driver may have left in a Nextcloud drive, which the profile's
  `trash ls` doesn't list (`trash restore` still takes them by id).

As built (pull request 7, § 14's row 7; `crates/core/src/api/owncloud.rs`, `crates/native/src/storage/profile/owncloud.rs`,
`crates/core/src/storage/probe.rs` and `probe/fake.rs`, `crates/native/src/storage/probe.rs`, `apps/craftcloud/src/cli/{account,remote}.rs`;
measured against ownCloud 10.15.3 on 2026-11-18, the answers recorded in `crates/core/tests/fixtures/owncloud-10.15/`):

- **ownCloud's profile** plugs into pull request 6's hook, the same shape as Nextcloud's, and knows: the files base
  `/remote.php/dav/files/<user id>/` (the user id from `current-user-principal`, the same request as Nextcloud's);
  `status.php` for detection and the version string (`productname` "ownCloud"; one more product for the parser);
  the trash bin at `/remote.php/dav/trash-bin/<user id>/` (Nextcloud's `trashbin/…/trash/` path answers 404), its
  items named by the **file id** they had (a folder with a trailing slash) with `oc:trashbin-original-filename`,
  `oc:trashbin-original-location` and `oc:trashbin-delete-timestamp`, a `GET` of an item answering 403 (only
  listings go there); a restore as a **`MOVE` of the item to its original path under the files base** with
  `Overwrite: F` (honoured: 412 onto a taken name and the file untouched, so a taken name gets
  `names::restored_name`; without the header the server overwrites; a missing parent answers 409, so the parents are
  made first; Nextcloud's `restore/` shape answers 409); versions at `/remote.php/dav/meta/<file id>/v/<seconds>`,
  the current file **not** among them (so step 17 lists before and after the overwrite and needs one more); a
  version restored as on Nextcloud, by the generic driver's guarded path (the file's current ETag, a `GET` of the
  version's bytes at `meta/<file id>/v/<seconds>`, a `PUT` with `If-Match` and the checksum check; a 412 is
  "changed meanwhile, nothing restored", exit code 5; the server keeps the replaced file as a version itself) and
  only for a stamp a fresh listing names. ownCloud's own restore, a `COPY` of the version over the file with
  `Overwrite: T`, is never sent: it has no precondition on the file it replaces, and since two writes within one
  second keep only one version (F1), a write landing just before it could be lost for good; the quota from WebDAV's `quota-available-bytes` on the
  base, `-3` read as no limit. `oc:checksums` (SHA1, MD5, ADLER32) are computed by the server and listed for every
  file, and an `OC-Checksum` sent with a `PUT` is checked against the bytes received (400 and nothing written on a
  mismatch); `Depth: infinity` answers 412 and a `PUT` into a missing folder 409, both recorded by the probe as
  before. **The probe on the trash root** gained one general hook for this: a known-server check's `Request` may
  say its destination is under the files base (`destination_under_base`), and the runner's `guarded` then allows a
  `MOVE` only of an item right under the root to exactly the cloud path the server said it came from, inside the
  probe's own folder, with `Overwrite: F` (measured: ownCloud moves a trashed item to **any** path under the files
  base, so the guard matters); a `DELETE` there stays refused (Q8).
- **Sign-in** is an app password the person makes in Settings › Security › "App passwords / tokens" and pastes in:
  `craftcloud login <server> --kind owncloud --user NAME` (the password from `CRAFTCLOUD_APP_PASSWORD` or a prompt,
  never an argument; without `--user` the command says where to make one). ownCloud 10 has no browser sign-in a
  third-party client can use without an admin (§ 10: OAuth2 is an admin's app, a bonus for later). Without
  `--kind`, and with the opt-in, a server whose `status.php` says ownCloud is signed in to as one. `logout` forgets
  the password and says where to revoke it (ownCloud 10 has no call to end an app password from the client).
- **The checksum check (Q13)** is pull request 6's, behind the profile's "verify writes by checksum": every `PUT`
  carries `OC-Checksum: SHA1`, which ownCloud verifies, and the read-back must list it. Measured on 10.15.3: three
  writes right behind each other shared one ETag for the two within the same second (as pull request 3 saw), and
  **the version slot named by that second kept only the later bytes**: the earlier bytes' version is lost
  (UPSTREAM.md F1; Nextcloud leaves no version at all, E1). The scripted server's ownCloud flavour (`Flags::owncloud`)
  lists a checksum for every file, so the driver and engine tests show the other client's write landing silently
  without the check and as a conflict copy with it, and the engine's request log of a cycle over the profile (every
  `PUT` with its checksum and one read-back, no `COPY`, the delete a `DELETE` with `If-Match` into the server's
  trash, nothing of CraftCloud's own on the server). Nothing in the engine changed: OxiCloud's golden logs stand.
- **The command line** on ownCloud: the same commands as on Nextcloud, through the trait; `trash ls` lists the
  server's trash by original path (the id is the file id), `trash restore` puts an item back by `MOVE` (`--as PATH`
  restores elsewhere, which ownCloud allows); `versions` and `versions … restore` are the server's by file id;
  `trash rm` and `trash empty` answer code 10 (Q8); `server check` says "ownCloud 10.15.3.0" and runs steps 16 and 17.
  The live suite's cleanup on ownCloud now reads the trash bin through the profile (pull request 5's cleanup used
  Nextcloud's path, which 404s there, so a run on ownCloud left its folders in the server's trash).
- **Left as is:** the generic driver's `upload` reads a 400 as "refused for what it is", so on ownCloud a file whose
  bytes change while it is being read (the checksum then mismatches) waits until it changes again, which it has;
  the desktop's sign-in (row 10); `--kind owncloud` has no `--no-browser` since there is no browser flow.

## 8. How each non-negotiable is met

| Non-negotiable | Server has it (honoured, measured) | Server lacks it | OpenCloud as measured |
|---|---|---|---|
| **Create only if absent** | `PUT` with `If-None-Match: *`; 412 → conflict copy | No two-way: download only | Ignored (reva#866) → download only (Q1) |
| **Overwrite only if unchanged** | `PUT` with `If-Match`; 412 → conflict copy | No two-way | Honoured, not shown to be atomic (reva#959); moot in download only |
| **Old bytes kept before an overwrite** | Server versions by file id (they follow renames; `move_versions` is a no-op) | Client versions: `COPY` into `.craftcloud/versions/<path>/<time>`, 5 kept, as today. Without `COPY` either: no two-way (Q2) | Server versions |
| **Delete only if unchanged** | `DELETE` (or the trash `MOVE`) with `If-Match` | Check, then move to a trash. The window is the requests between the check and the move, the same window OxiCloud has today: `still_at` lists the parent folder, then the file's REST id is looked up (`folder_id`, `child_id`, a few requests) and `files::trash` moves it. A change landing in that window goes to the trash with the file, never lost; acceptable only because the file lands in a trash | `DELETE` ignores `If-Match` (reva#961); moot in download only |
| **Deletes go to a trash, never for good** | Server trash, confirmed by the probe's round trip | Client trash: `MOVE` with `Overwrite: F` to `.craftcloud/trash/<time>-<device>/<path>` (the original path is in the name, for a restore); permanent `DELETE` only through `purge_own`, after the retention (Q5) | Server trash |
| **A folder trashed only if a fresh listing says it holds nothing new** | `cloud_holds` re-lists every level now, as today; any skipping of unchanged folders never applies to this listing | Same | Same |
| **A folder trashed as one item** | `DELETE` of the folder into the server trash | One `MOVE` of the folder into the client trash | Server trash |
| **Empty folders removed** | `DELETE` into the server trash | `MOVE` into the client trash: an empty-looking folder that gained a file meanwhile is kept whole, never deleted recursively | Server trash |
| **Listings: a failure is unknown** | Depth 1, four at once, every folder every cycle | Same; an href outside the base, a non-207 or unparsable XML fails the listing | Same |
| **Renames and moves are moves** | Paired by file id where present (state gains an optional `id`), else by ETag; `MOVE` with `Overwrite: F` | Without `move_no_overwrite`: no two-way | File ids |
| **One rule for what the sync skips** | `names::skipped` takes the server's `NameRules`; the same call for the local scan, the listing and the state. OxiCloud's rules are today's | Unknown refusals: the existing "4xx waits until the file changes" | Backslash refused; the rest from the probe |
| **Names that differ only in case** | Case-sensitive server: as today | Case-insensitive server: the cloud side gets the clash rule Windows clients use today (`names::clashes`) | Probe |
| **Mass deletes wait** | Unchanged (engine) | Unchanged | Unchanged |
| **A missing sync folder holds the sync** | Unchanged (engine) | Unchanged | Unchanged |
| **`.craftcloud/` never synced** | Unchanged; `.craftcloud/trash/` and `.craftcloud/probe/` are inside it | Unchanged | Unchanged |
| **Interrupted downloads resume only into the same version** | `Range` with the listed ETag, as today | Without `Range`: start over | Probe |
| **Quota** | 507 → wait, as today; free space per profile | Unknown: 507 only | WebDAV props |

## 9. Download only, in detail

Download only is the safe mode for a server that can't guard writes. It must still never lose a file, now on the
local side:

- `crates/core::reconcile::restrict(plan, Mode::DownloadOnly)` turns the plan into one with **no action that writes
  to the server**: no upload, move, trash or folder made there. A property test checks this for every plan.
- A file **changed here** is not sent. It stays as it is; the activity list says once "changed on this computer,
  not uploaded: this server only syncs downwards". If the cloud then changes it too, the local file becomes a
  **local** conflict copy (not uploaded) and the cloud version comes down.
- A file **deleted here** comes back from the cloud on the next cycle (the folder is a mirror of the cloud).
- A file **deleted in the cloud** is never deleted here for good, since the server may have no trash and this may
  be the last copy: after the mass-delete hold, it is **moved into `.craftcloud/trash/` in the local folder**
  (ignored by the sync, kept 30 days like the client trash, Q5, and shown by `craftcloud trash ls` beside the cloud's trash). A file
  changed here since the last sync stays where it is, as today.
- A folder whose **cloud listing failed** (an error, a non-207, an href outside the base) is unknown: everything
  under it is held, neither downloaded over nor taken as deleted, and the activity list says so, as in two-way
  sync.
- A file **new here** stays here and is listed once as not uploaded.

## 10. Sign-in and tokens

| Server | Sign-in | What the client stores | Sign-out |
|---|---|---|---|
| OxiCloud | Device flow, as today | Access and refresh tokens, as today | As today |
| Nextcloud | Login Flow v2: the client opens the browser, the person signs in there, the client polls and receives an app password made for this device | The login name and that app password | `DELETE` the app password (`/ocs/v2.php/core/apppassword`), then forget it |
| ownCloud | An app password the person makes in Settings › Security and pastes in. OAuth2 only where an admin has set it up for CraftCloud (later, a bonus) | Login name and app password | Forget it; the person revokes it on the server (the client says where) |
| OpenCloud | OIDC with PKCE in the browser, through the server's own identity provider. **To verify in its profile pull request**: which client registration a third-party desktop app can use without an admin, and whether app tokens are available to a person. Basic auth works only when an admin turns on `PROXY_ENABLE_BASIC_AUTH`, so it is not relied on | Tokens, like OxiCloud | Ends the session where the server allows |
| Other WebDAV | Address, user name and an app or WebDAV password (Q6) | User name and password | Forget it |

- **Storage.** The same `TokenStore` (system keychain, else the private file), keyed by server as today. What it
  stores becomes a `Credential`: `Bearer { access, refresh }` or `AppPassword { user, secret }`, serialised with a
  `kind` field that defaults to `bearer` when absent, so every existing entry reads unchanged and OxiCloud's
  entries keep today's exact shape (an older client can still read them).
- **Refresh.** `Session`'s refresh, lock file and take-over logic apply to `Bearer` only. An app password doesn't
  refresh; a 401 with one is "signed out" (it was revoked).
- **Never the account password.** The Connect sheet and `craftcloud login` ask for "an app or WebDAV password, not
  your account password", and where the server has a browser sign-in the client uses it instead of asking. The client can't
  verify which kind of password it was given; the wording and the flows are the guard.
- **HTTPS.** A password is never sent over plain HTTP, except to a loopback address (tests, a local server).
- **CLI.** The app password comes from a prompt or the `CRAFTCLOUD_APP_PASSWORD` environment variable, never from
  the command line (it would land in the shell history and the process list). It never reaches a log or an error.

## 11. Settings, desktop and CLI

### Settings

- `ClientSettings` gains `storage` (`oxicloud`, `nextcloud`, `owncloud`, `opencloud` or `webdav`; missing means
  `oxicloud`) and `dav_url` (empty for OxiCloud). Both are always written, since the settings store merges into
  the file.
- The probe's result lives in `storage-check.json` next to the settings, not in them.

### Desktop

- **Connect sheet.** The person types an address, as today. The client detects the kind of server (section 7.4)
  and says what it found ("Nextcloud 31 at cloud.example.org"). A link "Another kind of server" lets them choose
  CraftCloud, Nextcloud, ownCloud, OpenCloud or "Other WebDAV server" (which asks for the WebDAV address, the user
  name and an app or WebDAV password).
- **After the check.** Two-way: straight on to choosing folders, as today. Otherwise a panel before choosing
  folders, for example (wording for approval):
  - *Download only.* "This server overwrote a file it was asked to leave alone, so CraftCloud won't upload to it:
    files come down to this computer, and changes made here stay here. [What was checked] [Check again]"
  - *Not possible.* "CraftCloud can't sync with this server safely: it lists files without a version tag, so a
    change can't be told from a copy. [What was checked] [Check again]"
- **Status.** The window and the tray say "Download only" beside the state, so a person never mistakes it for a
  two-way sync.
- **Settings.** A "Server" section: the kind, the address, the mode and the date of the last check, "What was
  checked" (the probe table in plain words) and "Check again".
- **What isn't there.** "Open in the browser" opens the server's own web page. Storage use shows where the server
  gives it. Admin-only parts (recounting storage use) are OxiCloud only.

### Command line

- `craftcloud login cloud.example.org` detects the server as the desktop does; `--kind nextcloud|owncloud|opencloud|webdav`
  overrides, `--user NAME` names the user for app passwords.
- `craftcloud server check [--json]` runs the probe and prints each check and the mode.
- `craftcloud status` adds the mode.
- `ls`, `put`, `mv`, `rm`, `trash`, `versions`, `restore`, `sync`, `files`, `open` go through the trait.
- `share`, `server users`, `server info`'s OxiCloud fields, `starred`, `recent`, `search` answer "not available on
  this server" with a new exit code 10 and JSON kind `unsupported` (an addition; existing codes keep their meaning).

## 12. Migration and "upgrade all devices together"

- **OxiCloud users: no change.** Missing `storage` reads as `oxicloud`; keychain entries, `state.json` and the
  plans are unchanged. The refactor's pull request proves this with today's engine tests, the live suite against
  OxiCloud (21/21), the mixed-version two-client test (`cargo xtask e2e-sync` with the previous release on one
  side) and golden request logs (section 13): the same scripted cloud gets the same requests before and after.
- **state.json.** New fields (`id` on a file) are optional with a default, so 0.2.0 still reads a 0.3.0 state file
  (it ignores what it doesn't know), and a downgrade doesn't lose the sync state.
- **New servers.** No 0.2.0 client can sync with Nextcloud, ownCloud, OpenCloud or other WebDAV servers, so no
  older device can undo a 0.3.0 plan there. From 0.3.0 on, any change to how a server type is decided or what it
  skips gets its own "upgrade all devices together" line, as for OxiCloud.
- **RELEASE-NOTES for 0.3.0** say: new server types (desktop and command line only); OxiCloud unchanged; on a new
  server type, every device syncing that account must run 0.3.0 or later.
- **AGENTS.md's wording.** Both lines written for OxiCloud are amended in this change: the sign-in line (Q11) and
  the delete line (Q12), which now says how each server meets the rule (never a permanent delete of the person's
  files; a folder as one item).
- **A folder another client syncs.** If the chosen local folder holds another sync client's database
  (`.sync_*.db`, `.owncloudsync.log`, `.csync_journal.db`), the client refuses it: two engines on one folder fight.

## 13. Test plan

| Layer | What | Runs |
|---|---|---|
| `crates/core` unit | `mode_for` over a table of capabilities; the probe's verdict from scripted answers; `NameRules` and `skipped` with each server's rules | `cargo xtask ci` |
| `crates/core` property | Download-only plans never write to the server and never delete a local change; OxiCloud capabilities give exactly today's plan; file-id pairing never pairs two different ids | `cargo xtask ci` |
| `crates/native` driver | Each driver against `fake_server`: every write carries its precondition (a request without `If-Match`/`If-None-Match`/`Overwrite: F` fails the test); hrefs outside the base fail the listing; `purge_own` refuses a path outside `.craftcloud/` | `cargo xtask ci` |
| `crates/native` probe | Scripted servers that misbehave: answers 2xx to a stale `If-Match` → download only; ignores `If-None-Match` → download only; times out mid-probe → refused, and the probe folder is swept next time; no ETags → refused | `cargo xtask ci` |
| `crates/native` engine | Today's engine tests run once per driver; a runtime guard failure stops two-way sync | `cargo xtask ci` |
| `crates/native` golden logs | `crates/native/src/sync/golden_tests.rs` runs whole cycles of 15 scenarios against a small scripted OxiCloud (`fake_server`) and compares, line by line, every request (method, path, the guard headers, `Destination`, `Depth`, `Range`, the bearer token, the body or its hash), the activity lines, the sync state, the saved `state.json` and the final local folder and cloud with the logs in `crates/native/src/sync/golden/` (recorded before pull request 1; `CRAFTCLOUD_BLESS=1` records them again, only for a change meant to alter what goes over the wire). On every log it checks that every `PUT` carries `If-Match` or `If-None-Match: *`, every `MOVE` and `COPY` `Overwrite: F`, and a WebDAV `DELETE` only inside `.craftcloud/` | `cargo xtask ci` |
| Live | `crates/native/tests/live.rs` with `CRAFTCLOUD_TEST_KIND` (`oxicloud` default, `nextcloud`, `owncloud`, `opencloud`, `webdav`; every kind but the first only with `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`): the same tests, the cloud side (trash, versions, ids, cleanup) per kind behind the trait; per-server deadlines (Nextcloud is slower); the mode asserted on every pass; the two-way tests where two-way sync is expected, the download-only suite (section 9) where download only is | By hand (`tests/other-servers/README.md`), and in CI (below) |
| CLI live | `apps/craftcloud/tests/cli_live.rs` for `login --kind webdav`, `server check`, the file, trash and version commands, and the `unsupported` answers | Same |
| Windows | The 0.2.0 Windows checks once, against Nextcloud | By hand, before the release |

**Test servers.** `tests/other-servers/compose.yaml` (pull request 5) is the neutral compose file: loopback ports
only (`${…_PORT:-…}` variables, 8111 to 8115 by default), passwords from the environment (all required), every
image pinned by digest (OpenCloud at the stable release 9.1.1, never `rolling` or `latest`), the user `john`
everywhere but OpenCloud (its built-in `admin`), and the two plain WebDAV servers: `rclone serve webdav`, which
ignores `If-Match` and `If-None-Match` on `PUT` (golang/go#44492), for the download-only path, and Apache httpd
with `mod_dav`, which keeps every guard once its ETags are content digests (section 5), for the client's own
trash and versions. Its README says how to start one server at a time, how the app passwords are made, how each
suite runs and how to tear the set down (`down -v`). The servers are test containers, never shipped or linked.

The live tests make the app password the way a person would (Nextcloud's OCS app-password call; ownCloud's
Settings › Security through the web session), so the suite never signs in with the account password either,
except to create that app password (OpenCloud: its `auth-app` token call, `POST /auth-app/tokens`, on by
default in 9.1.1 and taking the account's basic auth through the test server's `PROXY_ENABLE_BASIC_AUTH`; the
token is then the WebDAV password). A plain WebDAV server's WebDAV password is the sign-in (Q6).

**CI.** A separate `👷 CI:` pull request adds an `other-servers` job (Linux, a matrix over the four kinds) beside
today's `live` job. The repository is public, so Linux minutes are free; it runs on pull requests that touch
`crates/core/src/{storage,reconcile,names}*`, `crates/native/src/{storage,sync}*` or the live tests, nightly, and
by hand. The OxiCloud `live` job keeps running on every pull request, unchanged.

## 14. Order of work

Small pull requests into `main`, each shippable, each leaving OxiCloud exactly as it was (its engine tests and the
live job green). From pull request 4 on, everything new is behind `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` (section 16)
until the 0.3.0 release pull request. Sizes are rough lines changed.

| # | Pull request | What | Size |
|---|---|---|---|
| 1 | `♻️ Sync: the engine talks to a storage trait; OxiCloud is its first driver` | `RemoteStorage`, `Capabilities` (OxiCloud's filled in by hand), today's code moved into `storage::oxicloud`. No behaviour change: golden-plan test, engine tests, live job | L, ~1,200 mostly moved |
| 2 | `✨ Sync: modes, and the plan cut to download only` | `Mode`, `mode_for`, `reconcile::restrict`, `NameRules` in `names::skipped` (OxiCloud's rules unchanged); property tests | M, ~500 |
| 3 | `✨ Sync: the server check` | The probe in core (steps, verdict) and its runner; `craftcloud server check` (OxiCloud reports its known capabilities, no probe there) | M, ~700 |
| 4 | `✨ Sync: any WebDAV server, with the client's own trash and versions` | The opt-in (`CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`) and its refusal tests, the generic driver, `Credential`, `storage`/`dav_url` settings, `craftcloud login --kind webdav`, `.craftcloud/trash/` with retention, the `unsupported` exit code; engine tests per driver | L, ~1,200 |
| 5 | `✅ Tests: the live suite against other servers` | `CRAFTCLOUD_TEST_KIND`, the compose file, the plain WebDAV server, download-only live tests | M, ~600 |
| 6 | `✨ Sync: Nextcloud` | Profile: Login Flow v2, current-user-principal, trashbin, versions by id, OCS quota, 404 for a missing parent | M, ~600 |
| 7 | `✨ Sync: ownCloud` | Profile: app passwords, trashbin, meta versions | S, ~300 |
| 8 | `✨ Sync: OpenCloud` | Profile: its sign-in (verified then), spaces trash restore, download only (Q1); UPSTREAM.md links reva#866, #959 and #961 | M, ~500 |
| 9 | `👷 CI: live tests against Nextcloud, ownCloud, OpenCloud and plain WebDAV` | The `other-servers` job | S, ~100 |
| 10 | `✨ Desktop: choosing a server, and what the check found` | Connect sheet detection and choice, the mode panel and status, Settings › Server; kittest shots compared with `design/` | M, ~800 |
| 11 | `📝 Docs: other servers` | ARCHITECTURE, CLI, STATUS, README, RELEASE-NOTES (0.3.0), ROADMAP M9 | S |

Pull requests 1 to 3 change nothing a person sees, with one exception: pull request 3 adds the informational
`craftcloud server check`, which on OxiCloud reports the known capabilities and writes nothing. 4 to 9 add server
types reachable from the command line, and 10 from the desktop, both only with the opt-in. The 0.3.0 release pull request removes the opt-in and turns them
on.

## 15. Risks

| Risk | Mitigation |
|---|---|
| The refactor changes OxiCloud's behaviour by accident | PR 1 moves code without changing it, a golden-plan test, the live job, `e2e-sync` with the previous release |
| A server passes the probe but misbehaves later (an upgrade, a proxy change) | Re-probe daily and on a version change; the runtime watch stops two-way sync at the first sign |
| A guard that is honoured but not atomic (OpenCloud's `MOVE`) | Profiles may lower a mode from measurements; never raise it |
| Nextcloud is slow (about 4 uploads a second) | Acceptable for 0.3.0; per-server live timeouts; parallel guarded uploads later (Q7) |
| Probe files in the person's server trash | One folder per probe, named clearly (`.craftcloud/probe/…`) |
| Client trash fills the person's storage | 30 days, flat (Q5); only the person empties it sooner; storage use counts it |
| A person gives the account password anyway | The wording and the browser sign-ins; nothing more can be checked |
| OpenCloud's sign-in for a third-party app needs an admin | Verified first in PR 8; if it needs an admin, OpenCloud waits, the rest ships |

## 16. Decisions

The maintainer took the recommendations below after a research pass confirmed them, and answered Q10 and Q12. No
decision is pending.

| # | Question | Decision | Why, and the evidence |
|---|---|---|---|
| Q1 | **OpenCloud: two-way or download only?** | **Download only**, until OpenCloud honours `If-None-Match: *` and its guards hold under a race | Its WebDAV layer forwards only `If-Match` on upload and never sets the CS3 `if_not_exist` option the storage API has (`ocdav/put.go` on opencloud-eu/reva `main`; `provider_api.proto`; cs3org/reva#1786). `If-None-Match` ignored: reva#866 (open, reproduced on OpenCloud 8.1.0). Two uploads with the same `If-Match` both got 201 in 10 of 10 rounds on 9.0.0: reva#959 (open; TUS uploads, plain `PUT` not measured under a race). `MOVE` and `DELETE` ignore a stale `If-Match`: reva#961 (open; `DELETE` confirmed by a maintainer on 9.0.0, 14 November). The spike's workaround (temporary name, `MOVE` with `Overwrite: F`, `oc:checksums` SHA1) detects a lost race after the fact; it is not a guarded create. UPSTREAM.md links these issues; nothing new is filed |
| Q2 | **A server with neither `COPY` nor its own versions** | **Download only** | Uploading a snapshot from the last synced bytes would add a download before every save. Note that `COPY` alone is not enough: `rclone serve webdav` (Go's `x/net/webdav`) has `COPY` but ignores `If-Match` and `If-None-Match` on `PUT` (golang/go#44492, open; the TODO in `webdav.go`), so it stops at the guard steps and gets download only. nginx's `dav` module appears not to check `If-*` either (read in its code, not tested). Nothing in this design promises two-way sync with either |
| Q3 | **Probe OxiCloud too?** | **No, for 0.3.0** | No change for OxiCloud users; revisit after 0.3.0 |
| Q4 | **Skip folders whose ETag didn't change?** | **No**: every folder is listed every cycle, on every server | Folder ETags miss changes in practice, not only in theory: Nextcloud documents that changes made directly on external storage update no ETag; OpenCloud's storage missed on-disk changes until reva PRs #808 and #809; OxiCloud's folder ETags didn't always change (UPSTREAM.md C8). A single probe can't prove such faults absent, and trash decisions re-list anyway |
| Q5 | **How long the client trash keeps things** | **30 days, flat.** Only the person empties it sooner (`craftcloud trash empty`, "Empty trash") | Deliberately **not** Nextcloud's and ownCloud's default of deleting sooner when the quota runs low: that is an automatic permanent delete of the person's files |
| Q6 | **Plain WebDAV servers with no app passwords** | **Accepted, for "Other WebDAV" only**, asked for as "an app or WebDAV password"; HTTPS required (loopback excepted) | Hosted WebDAV services make these separate passwords themselves: Koofr and Fastmail require an app password for WebDAV; Seafile has a separate WebDAV password, which its admin switches on. Synology has none (a dedicated WebDAV user is the way; sources disagree, unverified). Nextcloud, ownCloud and OpenCloud never take this path |
| Q7 | **Nextcloud's speed** (about 4 uploads a second) | **Accepted for 0.3.0** | The faster upload paths, bulk upload (`/remote.php/dav/bulk`) and chunked upload v2, carry no preconditions, so the trait rules them out for good. The only safe speed-up is guarded `PUT`s in parallel (Nextcloud's own desktop client runs 6, `OWNCLOUD_MAX_PARALLEL`): a later change with its own tests |
| Q8 | **Probe leftovers in the server trash** | **Accepted**: one folder per probe, never purged | Purging a person's trash is a permanent delete outside `.craftcloud/`. The leftovers expire from Nextcloud's and ownCloud's trash with their 30-day default |
| Q9 | **SHA1 besides SHA-256** in the sync state | **Not now** | Not needed without the OpenCloud workaround (Q1). For later: ocdav accepts an `OC-Checksum` header on upload, a check by the server that the bytes arrived whole |
| Q10 | **Where the work lands** | **On `main`**, one shippable pull request at a time, the new server kinds hidden and refused without an explicit opt-in until the 0.3.0 release pull request turns them on (below) | Pull requests 1 to 3 rewrite `crates/native/src/sync.rs`; on a long-lived branch every 0.2.x sync fix landing meanwhile would have to be merged by hand into moved code. The opt-in keeps a 0.2.x hotfix tagged from `main` from shipping `--kind webdav` early |
| Q11 | **AGENTS.md said the client never sees a password** | **Amended in this change** to: "The client never asks for or stores your account password: it signs in through the server's browser sign-in, or with an app password you make on the server." (modelled on Nextcloud's public Login Flow docs) | It is already true of today's client (device flow only), and it binds the pull requests that follow |
| Q12 | **AGENTS.md's "WebDAV `DELETE` is permanent" rule** was written for OxiCloud only | **Amended in this change** to: "A delete never removes a file for good: it goes to the server's own trash (OxiCloud: over REST; Nextcloud and ownCloud: their WebDAV `DELETE`, which the check on connect confirms lands in the trash), or to `.craftcloud/trash/` where the server has none. A folder goes as one item, so a restore brings it back whole." | It changes a non-negotiable, so it was decided on its own, not folded into Q11. The rule's meaning stays (never a permanent delete of the person's files; a folder as one item); the wording now says how each server meets it |
| Q13 | **Nextcloud and ownCloud reuse a file's ETag within one second** (Jonas, 2026-11-18): two writes to one file within the same second can't be told apart by `If-Match`, so a second client's write could land under the first client's tag and be overwritten by its next edit. Two-way sync or not? | **Two-way stays on, and the window is closed by a checksum check.** On these profiles every `PUT` (create and overwrite) carries `OC-Checksum: SHA1:<hex>`; afterwards the server's `oc:checksums` of the file must be that checksum, else the server holds someone else's write under our tag and the engine makes a **conflict copy** of the local bytes, never a silent overwrite, and says so. The driver does the check behind the profile's "verify writes by checksum" switch, so ownCloud (row 7) shares the code | Pull request 3 measured it with `curl`: Nextcloud 31 and ownCloud 10.15 keep the tag while the file's modified time in whole seconds is unchanged (3 of 3 rounds each; the file-cache scanner derives the tag from the storage mtime). The probe already waits a second between its own overwrites; the engine can't wait for other clients. Nextcloud lists `oc:checksums` only for a file whose last `PUT` carried `OC-Checksum`, and stores what it is sent without checking it (measured on 31.0.14), so the header goes on every `PUT` and a mismatch, or no checksum at all, means another write. A test against the scripted server reusing the tag shows the write landing silently without the check and a conflict copy with it. UPSTREAM.md E1 proposes a fresh ETag on every write; nothing is filed |

### The opt-in until 0.3.0 (Q10)

The work lands on `main`, and a 0.2.x hotfix may be tagged from `main` at any time. So until the 0.3.0 release the
new server kinds are switched off in every build unless the person running it switches them on:

- **The switch** is the environment variable `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`, read in one place
  (`crates/native`) and used by the command line, the daemon and the desktop client alike. Only the exact value `1`
  counts. Nothing in a released build, its packaging or its settings sets it, so a hotfix can't switch it on by
  accident, and a person can't do so by a stray flag or a click.
- **Without it:** `--kind` accepts only `oxicloud` and is not in `--help`; `server check` reports OxiCloud only;
  detection never suggests another server kind; the Connect sheet offers no "Another kind of server". A settings
  file that names another kind (left from a test with the switch on) is **refused** with "This build syncs only
  with a CraftCloud server; set CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 to try other servers": it never falls back to
  treating that server as OxiCloud, and nothing is synced.
- **Added by** pull request 4 (the first that adds a server kind), with a test that a build without the variable
  refuses `--kind webdav` and a settings file naming it. Pull requests 5 to 11 put everything new behind it.
- **Removed by** the 0.3.0 release pull request: the variable is gone, the kinds are offered, and its tests turn
  into tests that they are.

## 17. Sources

Measured by hand and with the spike on 2026-11-16 (section 5), and:

- OpenCloud and reva:
  - reva#866, `If-None-Match` ignored on uploads: https://github.com/opencloud-eu/reva/issues/866
  - reva#959, concurrent conditional uploads both succeed: https://github.com/opencloud-eu/reva/issues/959
  - reva#961, `MOVE` and `DELETE` ignore a stale `If-Match`: https://github.com/opencloud-eu/reva/issues/961
  - reva#883, overlapping uploads and broken versions: https://github.com/opencloud-eu/reva/issues/883
  - `ocdav/put.go`: https://github.com/opencloud-eu/reva/blob/main/internal/http/services/owncloud/ocdav/put.go
  - CS3 `provider_api.proto`: https://github.com/cs3org/cs3apis/blob/main/cs3/storage/provider/v1beta1/provider_api.proto
  - cs3org/reva#1786, exclusive uploads: https://github.com/cs3org/reva/issues/1786
  - opencloud#3736, concurrent `MOVE`, `DELETE` and `PUT` races: https://github.com/opencloud-eu/opencloud/issues/3736
  - OpenCloud releases 9.1.1 and 9.2.0: https://github.com/opencloud-eu/opencloud/releases/tag/v9.1.1,
    https://github.com/opencloud-eu/opencloud/releases/tag/v9.2.0
  - reva PRs #808 and #809, on-disk changes picked up: https://github.com/opencloud-eu/reva/pull/808,
    https://github.com/opencloud-eu/reva/pull/809
- Go and rclone, nginx:
  - golang/go#44492: https://github.com/golang/go/issues/44492
  - `x/net/webdav`, `webdav.go`: https://github.com/golang/net/blob/master/webdav/webdav.go
  - `rclone serve webdav`: https://rclone.org/commands/rclone_serve_webdav/
  - nginx `dav` module: https://nginx.org/en/docs/http/ngx_http_dav_module.html
- Nextcloud:
  - External storage, changes made outside Nextcloud: https://docs.nextcloud.com/server/latest/admin_manual/configuration_files/external_storage_configuration_gui.html
  - Bulk upload: https://docs.nextcloud.com/server/latest/developer_manual/client_apis/WebDAV/bulkupload.html
  - Chunked upload: https://docs.nextcloud.com/server/latest/developer_manual/client_apis/WebDAV/chunking.html
  - Desktop client environment variables: https://docs.nextcloud.com/desktop/latest/envvars.html
  - Login Flow: https://docs.nextcloud.com/server/latest/developer_manual/client_apis/LoginFlow/index.html
  - Trash bin configuration: https://docs.nextcloud.com/server/latest/admin_manual/configuration_files/trashbin_configuration.html
- ownCloud trash bin options: https://doc.owncloud.com/server/next/admin_manual/configuration/files/trashbin_options.html
- App and WebDAV passwords elsewhere:
  - Koofr: https://koofr.eu/help/koofr_with_webdav/how-do-i-connect-a-service-to-koofr-through-webdav/
  - Fastmail: https://www.fastmail.help/hc/en-us/articles/1500000277882
  - Seafile settings: https://manual.seafile.com/latest/config/seahub_settings_py/
