# Any WebDAV server as the storage: design

Status: **design only, for approval before any code** (milestone M9, planned for 0.3.0). Today the desktop client
and the `craftcloud` command line sync only with CraftCloud's own server (OxiCloud). This document says how they
can sync with other WebDAV servers (Nextcloud, ownCloud, OpenCloud, a NAS, `rclone serve webdav` …) without ever
making the sync less safe.

It builds on the engine described in [ARCHITECTURE.md › Sync](ARCHITECTURE.md#sync) and on the sync
non-negotiables in [AGENTS.md](../AGENTS.md), which bind every part of it. Code references are to
`crates/core/src/{reconcile,names,sync}.rs`, `crates/core/src/api/dav.rs` and
`crates/native/src/{sync,session,http,secrets}.rs`.

The facts about the other servers below were **measured on 2026-11-16**: by hand with `curl` against Nextcloud 31,
ownCloud 10.15 and OpenCloud (rolling), and with a throwaway spike branch that made today's engine talk plain
WebDAV to all three and ran the live sync suite against them. The spike is not for merging; section 4 lists what
it taught.

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
- Files on demand (its own design, [FILES-ON-DEMAND.md](FILES-ON-DEMAND.md)), delta sync, chunked uploads.
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
| Upstream gaps go in UPSTREAM.md | AGENTS.md | OpenCloud's ignored `If-None-Match: *` and non-atomic `MOVE` get entries there, not issues filed without asking |
| No panics, no `unsafe` | Rust rule 1 | All server answers are hostile input: hrefs outside the base, missing ETags, odd status codes |
| Shared libraries first | Core rule 16 | WebDAV requests and multistatus parsing go through `craft-dav` from `storytold/craft-libs`, as today; what it lacks (extra namespaces such as `oc:`, `nc:`, trash and version properties) is proposed there rather than parsed by hand here. Server profiles stay in CraftCloud |
| Desktop sign-in through the browser; the client never sees a password | AGENTS.md | Kept for every server with a browser sign-in. An app password the person makes and pastes in (ownCloud, other WebDAV) is a password the client sees, so that line of AGENTS.md needs a maintainer's change in the pull request that adds it (Q11) |
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
| Create a file on OpenCloud | Upload under a temporary name, `MOVE` with `Overwrite: F`, then compare `oc:checksums` SHA1 | An open question (section 16, Q1); not two-way by default |
| Find a folder's id | `oc:fileid` | Entries carry an optional file id; moves pair by it where present |

## 5. What the servers do (measured on 2026-11-16)

| Behaviour | OxiCloud 0.10.0 | Nextcloud 31 | ownCloud 10.15 | OpenCloud |
|---|---|---|---|---|
| `PUT` with `If-None-Match: *` over an existing file | 412 | 412 | 412 | **ignored: overwrites** |
| `PUT` with a stale `If-Match` | 412 | 412 | 412 | 412 |
| `PUT`'s ETag equals `PROPFIND`'s | yes | yes | yes | yes |
| ETag changes on every write | yes | yes | yes | yes |
| ETag survives `MOVE` | yes | yes | yes | yes |
| Folder ETags follow their contents | **not reliably** (UPSTREAM C8) | yes | yes | yes |
| `oc:fileid`, stable across `MOVE` | no (404) | yes | yes | yes |
| `COPY` and `MOVE` | yes | yes | yes | yes |
| `MOVE`/`COPY` with `Overwrite: F` onto an existing name | 412 | 412 | 412 | 412, but **not atomic** under two creates at once (both won in 1 of 3 runs) |
| `DELETE` with a stale `If-Match` | (REST trash is used) | 412 | 412 | **ignored: deletes** |
| Where WebDAV `DELETE` goes | gone for good | server trash | server trash | server trash |
| Restore from the trash | REST | `MOVE` to `trashbin/<user>/restore/<name>` | `MOVE` to the original path | `MOVE`, only through its `spaces/<space>/` URL |
| Versions | none (CraftCloud keeps them in `.craftcloud/versions/`) | per file id, follow renames (`/remote.php/dav/versions/<user>/versions/<fileid>/`) | per file id (`/remote.php/dav/meta/<fileid>/v/`) | per file id (`/remote.php/dav/meta/<fileid>/v/`) |
| `oc:checksums` | no | when the client sent one | yes | yes |
| Quota | REST `/api/auth/me` | OCS user endpoint (WebDAV says `-3`) | WebDAV quota props | WebDAV quota props |
| `PUT` into a missing folder | 409 | **404** | 409 | 409 |
| `Depth: infinity` | 403 | 207 | 412 | 400 |
| Backslash in a name | refused | 400 | 400 | 400 |
| Browser sign-in | OAuth device flow | Login Flow v2 | OAuth2 app (off by default) | OIDC |
| App passwords made by the person | no (device flow instead) | yes | yes | to verify |

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

```rust
pub trait RemoteStorage: Send {
    fn capabilities(&self) -> &Capabilities;

    /// One folder, one level. An error is "unknown": the caller holds that place, never takes it as empty.
    fn list(&mut self, folder: &str) -> Result<Vec<Entry>, StorageError>;
    fn stat(&mut self, path: &str) -> Result<Option<Entry>, StorageError>;
    fn download(&mut self, path: &str, resume: Option<Resume>, out: &mut dyn Write) -> Result<Fetched, StorageError>;

    /// Only if nothing is there. `Written::Conflict` when something is.
    fn create(&mut self, path: &str, file: &Path) -> Result<Written, StorageError>;
    /// Only if the server still has `if_match`. `Written::Conflict` otherwise.
    fn replace(&mut self, path: &str, if_match: &Etag, file: &Path) -> Result<Written, StorageError>;
    fn make_folder(&mut self, path: &str) -> Result<(), StorageError>;
    /// Never onto an existing name; only if the source is still `guard`.
    fn move_to(&mut self, from: &str, to: &str, guard: MoveGuard) -> Result<Moved, StorageError>;
    /// Into a trash, never for good. A file only if still at its ETag; a folder only if a fresh
    /// listing holds exactly the files given.
    fn trash(&mut self, path: &str, guard: TrashGuard) -> Result<Trashed, StorageError>;
    fn trash_list(&mut self) -> Result<Vec<TrashItem>, StorageError>;
    fn restore(&mut self, item: &TrashItem) -> Result<(), StorageError>;

    /// Keep the version about to be replaced (no-op where the server keeps it itself).
    fn keep_version(&mut self, path: &str, current: &Etag) -> Result<(), StorageError>;
    fn move_versions(&mut self, from: &str, to: &str) -> Result<(), StorageError>;
    fn free_space(&mut self) -> Result<Option<u64>, StorageError>;

    /// Permanent removal, only inside CraftCloud's own `.craftcloud/` (probe files, old client
    /// trash, pruned versions). `OwnPath` can only be made for paths under `.craftcloud/`.
    fn purge_own(&mut self, path: &OwnPath) -> Result<(), StorageError>;
}
```

- `Entry` is today's `dav::Entry` plus an optional `file_id` and an optional `checksum`.
- `Written` is `Saved { etag, file_id }` or `Conflict`. A 412, or (where creates are emulated) a checksum that isn't
  ours, is a `Conflict`; the engine makes a conflict copy, as today.
- `StorageError` keeps today's distinctions: offline, signed out, no room (507), refused for what it is (4xx, waits
  until the file changes), and other.
- The engine's existing rails (mass-delete hold, unreadable places held, the two-second settle, part files and
  resume) stay in the engine and don't move into drivers.

### 7.3 Capabilities and modes

`Capabilities` is what the probe measured, filled in by the profile where the probe can't measure it (addresses,
how sign-in works):

| Field | Values | Measured by |
|---|---|---|
| `create_if_absent` | honoured · ignored · emulated (see Q1) | probe |
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
| Detected by | `GET /api/auth/status` | `GET /status.php` (`productname`) | `GET /status.php` | its well-known config | the person chooses it |
| WebDAV base | `/webdav/` | `/remote.php/dav/files/<user id>/` | `/remote.php/dav/files/<user id>/` | to verify (personal space URL) | the URL the person gives |
| User id | from the token | `current-user-principal` (the login name can differ) | `current-user-principal` | to verify | from the URL |
| Trash | REST (today) | WebDAV trashbin | WebDAV trashbin | spaces trash | client |
| Versions | client | server, by file id | server, by file id | server, by file id | client |
| Quota | REST | OCS user endpoint | WebDAV props | WebDAV props | WebDAV props, else unknown |
| Sign-in | device flow | Login Flow v2 | app password | OIDC (to verify) | app password |

The OxiCloud driver is today's code moved behind the trait. Nextcloud, ownCloud and OpenCloud use the
generic WebDAV driver with their profile.

### 7.5 The probe

**Where.** In `.craftcloud/probe/<device>-<random>/` on the server, so two devices probing at once never meet. It
starts by sweeping probe folders older than a day (a crashed probe), the way stale part files are swept.

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

## 8. How each non-negotiable is met

| Non-negotiable | Server has it (honoured, measured) | Server lacks it | OpenCloud as measured |
|---|---|---|---|
| **Create only if absent** | `PUT` with `If-None-Match: *`; 412 → conflict copy | No two-way: download only | Ignored → download only, unless Q1 |
| **Overwrite only if unchanged** | `PUT` with `If-Match`; 412 → conflict copy | No two-way | Honoured |
| **Old bytes kept before an overwrite** | Server versions by file id (they follow renames; `move_versions` is a no-op) | Client versions: `COPY` into `.craftcloud/versions/<path>/<time>`, 5 kept, as today. Without `COPY` either: no two-way (Q2) | Server versions |
| **Delete only if unchanged** | `DELETE` (or the trash `MOVE`) with `If-Match` | Check, then move to a trash: a one-request window, the same as OxiCloud's REST trash today, acceptable only because the file lands in a trash | `DELETE` ignores `If-Match`: check, then `DELETE` into its server trash |
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
- A file **deleted in the cloud** is deleted here, through the mass-delete hold, unless it changed here (then it
  stays, as today).
- A file **new here** stays here and is listed once as not uploaded.

## 10. Sign-in and tokens

| Server | Sign-in | What the client stores | Sign-out |
|---|---|---|---|
| OxiCloud | Device flow, as today | Access and refresh tokens, as today | As today |
| Nextcloud | Login Flow v2: the client opens the browser, the person signs in there, the client polls and receives an app password made for this device | The login name and that app password | `DELETE` the app password (`/ocs/v2.php/core/apppassword`), then forget it |
| ownCloud | An app password the person makes in Settings › Security and pastes in. OAuth2 only where an admin has set it up for CraftCloud (later, a bonus) | Login name and app password | Forget it; the person revokes it on the server (the client says where) |
| OpenCloud | OIDC with PKCE in the browser, through the server's own identity provider. **To verify in its profile pull request**: which client registration a third-party desktop app can use without an admin, and whether app tokens are available to a person. Basic auth works only when an admin turns on `PROXY_ENABLE_BASIC_AUTH`, so it is not relied on | Tokens, like OxiCloud | Ends the session where the server allows |
| Other WebDAV | Address, user name and an app password (or a WebDAV-only password, Q6) | User name and password | Forget it |

- **Storage.** The same `TokenStore` (system keychain, else the private file), keyed by server as today. What it
  stores becomes a `Credential`: `Bearer { access, refresh }` or `AppPassword { user, secret }`, serialised with a
  `kind` field that defaults to `bearer` when absent, so every existing entry reads unchanged and OxiCloud's
  entries keep today's exact shape (an older client can still read them).
- **Refresh.** `Session`'s refresh, lock file and take-over logic apply to `Bearer` only. An app password doesn't
  refresh; a 401 with one is "signed out" (it was revoked).
- **Never the account password.** The Connect sheet and `craftcloud login` say "an app password, not your account
  password", and where the server has a browser sign-in the client uses it instead of asking. The client can't
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
  name and an app password).
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
  side) and a golden test: the same scripted cloud gives the same plan before and after.
- **state.json.** New fields (`id` on a file) are optional with a default, so 0.2.0 still reads a 0.3.0 state file
  (it ignores what it doesn't know), and a downgrade doesn't lose the sync state.
- **New servers.** No 0.2.0 client can sync with Nextcloud, ownCloud, OpenCloud or other WebDAV servers, so no
  older device can undo a 0.3.0 plan there. From 0.3.0 on, any change to how a server type is decided or what it
  skips gets its own "upgrade all devices together" line, as for OxiCloud.
- **RELEASE-NOTES for 0.3.0** say: new server types (desktop and command line only); OxiCloud unchanged; on a new
  server type, every device syncing that account must run 0.3.0 or later.
- **AGENTS.md's wording.** Two non-negotiables are written for OxiCloud: "WebDAV `DELETE` is permanent; deletes go to
  the cloud's trash over REST" and "the client never sees a password". Their rule stays (never a permanent delete
  of the person's files; never the account password); the pull requests that add other servers reword them to say
  how each server meets it (Q11), with the maintainer's approval.
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
| Live | `crates/native/tests/live.rs` with `CRAFTCLOUD_TEST_KIND` (`oxicloud` default, `nextcloud`, `owncloud`, `opencloud`, `webdav`): the same 21 tests, the server-specific helpers (trash, versions, ids) behind the trait; per-server timeouts (Nextcloud is slower); plus a download-only suite against a server that fails the probe | By hand, and in CI (below) |
| CLI live | `apps/craftcloud/tests/cli_live.rs` for `login --kind webdav`, `server check`, and the `unsupported` answers | Same |
| Windows | The 0.2.0 Windows checks once, against Nextcloud | By hand, before the release |

**Test servers.** A neutral compose file goes into the repository with the live-test pull request
(`tests/other-servers/compose.yaml`): loopback ports only, passwords from the environment, images pinned by digest
there (the spike ran OpenCloud's rolling image, which must not be used in CI), and the user `john`:

```yaml
# Throwaway servers for CraftCloud's live tests. Loopback only; never for real data.
services:
  nextcloud:
    image: nextcloud:31-apache            # pinned by digest in the pull request
    ports: ["127.0.0.1:8095:80"]
    environment:
      SQLITE_DATABASE: nextcloud
      NEXTCLOUD_ADMIN_USER: john
      NEXTCLOUD_ADMIN_PASSWORD: ${NC_PASS:?}
      NEXTCLOUD_TRUSTED_DOMAINS: "127.0.0.1 localhost"
  owncloud:
    image: owncloud/server:10.15          # pinned by digest
    ports: ["127.0.0.1:8096:8080"]
    environment:
      OWNCLOUD_DOMAIN: 127.0.0.1:8096
      OWNCLOUD_TRUSTED_DOMAINS: 127.0.0.1,localhost
      OWNCLOUD_DB_TYPE: sqlite
      OWNCLOUD_ADMIN_USERNAME: john
      OWNCLOUD_ADMIN_PASSWORD: ${OC_PASS:?}
  opencloud:
    image: opencloudeu/opencloud:<release>  # a stable release, pinned by digest
    entrypoint: ["/bin/sh", "-c", "opencloud init || true; exec opencloud server"]
    ports: ["127.0.0.1:8097:9200"]
    environment:
      OC_URL: https://localhost:8097
      OC_INSECURE: "true"
      PROXY_TLS: "false"
      IDM_CREATE_DEMO_USERS: "false"
      IDM_ADMIN_PASSWORD: ${OPC_PASS:?}
  webdav:
    # A plain WebDAV server with no trash and no versions, to test the client's own
    # (rclone serve webdav or Apache mod_dav; which one passes the probe is measured then).
```

The live tests make the app password the way a person would (Nextcloud's and ownCloud's app-password API for the
test user), so the suite never signs in with the account password either, except to create that app password.

**CI.** A separate `👷 CI:` pull request adds an `other-servers` job (Linux, a matrix over the four kinds) beside
today's `live` job. The repository is public, so Linux minutes are free; it runs on pull requests that touch
`crates/core/src/{storage,reconcile,names}*`, `crates/native/src/{storage,sync}*` or the live tests, nightly, and
by hand. The OxiCloud `live` job keeps running on every pull request, unchanged.

## 14. Order of work

Small pull requests, each shippable, each leaving OxiCloud exactly as it was (its engine tests and the live job
green). Sizes are rough lines changed.

| # | Pull request | What | Size |
|---|---|---|---|
| 1 | `♻️ Sync: the engine talks to a storage trait; OxiCloud is its first driver` | `RemoteStorage`, `Capabilities` (OxiCloud's filled in by hand), today's code moved into `storage::oxicloud`. No behaviour change: golden-plan test, engine tests, live job | L, ~1,200 mostly moved |
| 2 | `✨ Sync: modes, and the plan cut to download only` | `Mode`, `mode_for`, `reconcile::restrict`, `NameRules` in `names::skipped` (OxiCloud's rules unchanged); property tests | M, ~500 |
| 3 | `✨ Sync: the server check` | The probe in core (steps, verdict) and its runner; `craftcloud server check` (OxiCloud reports its known capabilities, no probe there) | M, ~700 |
| 4 | `✨ Sync: any WebDAV server, with the client's own trash and versions` | The generic driver, `Credential`, `storage`/`dav_url` settings, `craftcloud login --kind webdav`, `.craftcloud/trash/` with retention, the `unsupported` exit code; engine tests per driver | L, ~1,200 |
| 5 | `✅ Tests: the live suite against other servers` | `CRAFTCLOUD_TEST_KIND`, the compose file, the plain WebDAV server, download-only live tests | M, ~600 |
| 6 | `✨ Sync: Nextcloud` | Profile: Login Flow v2, current-user-principal, trashbin, versions by id, OCS quota, 404 for a missing parent | M, ~600 |
| 7 | `✨ Sync: ownCloud` | Profile: app passwords, trashbin, meta versions | S, ~300 |
| 8 | `✨ Sync: OpenCloud` | Profile: its sign-in (verified then), spaces trash restore, the mode decided in Q1; UPSTREAM.md entries for `If-None-Match` and `MOVE` | M, ~500 |
| 9 | `👷 CI: live tests against Nextcloud, ownCloud, OpenCloud and plain WebDAV` | The `other-servers` job | S, ~100 |
| 10 | `✨ Desktop: choosing a server, and what the check found` | Connect sheet detection and choice, the mode panel and status, Settings › Server; kittest shots compared with `design/` | M, ~800 |
| 11 | `📝 Docs: other servers` | ARCHITECTURE, CLI, STATUS, README, RELEASE-NOTES (0.3.0), ROADMAP M9 | S |

Pull requests 1 to 3 change nothing a person sees; 4 to 9 add server types reachable from the command line;
10 makes them reachable from the desktop.

## 15. Risks

| Risk | Mitigation |
|---|---|
| The refactor changes OxiCloud's behaviour by accident | PR 1 moves code without changing it, a golden-plan test, the live job, `e2e-sync` with the previous release |
| A server passes the probe but misbehaves later (an upgrade, a proxy change) | Re-probe daily and on a version change; the runtime watch stops two-way sync at the first sign |
| A guard that is honoured but not atomic (OpenCloud's `MOVE`) | Profiles may lower a mode from measurements; never raise it |
| Nextcloud is slow (about 4 uploads a second) | Acceptable for 0.3.0; per-server live timeouts; parallel uploads later (Q7) |
| Probe files in the person's server trash | One folder per probe, named clearly (`.craftcloud/probe/…`) |
| Client trash fills the person's storage | Retention (Q5); `craftcloud trash empty` empties it; storage use counts it |
| A person gives the account password anyway | The wording and the browser sign-ins; nothing more can be checked |
| OpenCloud's sign-in for a third-party app needs an admin | Verified first in PR 8; if it needs an admin, OpenCloud waits, the rest ships |

## 16. Open questions (for the maintainer)

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q1 | **OpenCloud: two-way or download only?** It ignores `If-None-Match: *`. The spike's workaround (upload under a temporary name, `MOVE` with `Overwrite: F`, then compare `oc:checksums` SHA1) passed 21/21, but its `MOVE` isn't atomic: two devices creating the same new name within the same moment can both "win", and the earlier one's bytes then survive only if OpenCloud kept them as a version, which wasn't measured. That's detection after the fact, not a guarded create. | (a) Download only until OpenCloud honours `If-None-Match: *`; (b) two-way through the workaround, marked experimental | **(a)**, and an UPSTREAM.md entry with the reproduction. (b) can follow if a version is shown to be kept on the losing `MOVE` |
| Q2 | **A server with neither `COPY` nor its own versions:** old bytes can't be kept server-side before an overwrite. | (a) Download only; (b) two-way, uploading the snapshot from the last synced bytes (a download before every overwrite) | **(a)**. Rare (most WebDAV servers support `COPY`); (b) doubles the traffic for every save |
| Q3 | **Probe OxiCloud too?** It would catch a proxy in front that strips the guards. | (a) Never; (b) once, silently, lowering the mode only on failure | **(a) for 0.3.0** (no change for OxiCloud users); revisit after 0.3.0 |
| Q4 | **Skip folders whose ETag didn't change**, where the probe says folder ETags follow their contents? | (a) No, list every folder every cycle, everywhere; (b) yes, with a full listing every few minutes | **(a) for 0.3.0.** A single probe can't prove a fault that happens "not always"; trash decisions always re-list anyway |
| Q5 | **How long the client trash keeps things** (servers without a trash) | 7, 30 days, or until emptied | **30 days**, emptied by `craftcloud trash empty` or "Empty trash" |
| Q6 | **Plain WebDAV servers with no app passwords** (a NAS, `rclone serve webdav`): the only credential is a password. | (a) Accept a password for "Other WebDAV" only, worded "a WebDAV password (use a separate user if the server can make one)", HTTPS required; (b) refuse such servers | **(a)**: a dedicated WebDAV user is the NAS's app password. Nextcloud, ownCloud and OpenCloud never take this path |
| Q7 | **Nextcloud's speed** (about 4 uploads a second) | (a) Accept for 0.3.0; (b) parallel uploads now | **(a)**; parallel uploads are their own change with their own tests |
| Q8 | **Probe leftovers in the server trash** on known servers | (a) Accept one folder per probe; (b) also purge it from the server trash | **(a)**: purging from a person's trash is a permanent delete outside `.craftcloud/` |
| Q9 | **SHA1 besides SHA-256** in the sync state, to compare against `oc:checksums` (ownCloud, OpenCloud) and tell "same bytes" without a download | (a) Not now; (b) store both | **(a)**, unless Q1 is (b), which needs it |
| Q10 | **Where the work lands** | (a) `main`, new server types unreachable until PR 4/10; (b) a long-lived `feature/webdav-support` branch | **(a)** after this design: each pull request is shippable, and a long branch drifts from the sync fixes landing on `main` |
| Q11 | **AGENTS.md says the desktop client never sees a password.** App passwords pasted in (ownCloud, other WebDAV, and Q6) break that sentence as written, though the M9 decision allows them. | (a) Amend it: "never the account password; an app password the person makes, or the server's browser sign-in"; (b) browser sign-in only, so ownCloud and plain WebDAV wait | **(a)**, worded in the pull request that adds the generic driver (PR 4) |
