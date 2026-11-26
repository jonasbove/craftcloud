# Files on demand: design

Status: design only. v1 of the desktop client is a **full sync folder**: every file in the chosen folders is
downloaded and kept on disk. Files that download only when opened ("files on demand", "placeholder files") are
**not in v1**. This document is the design for that later feature, so the v1 code does not close doors it needs.

It builds on the v1 engine described in [ARCHITECTURE.md](ARCHITECTURE.md) and on the server gaps in
[UPSTREAM.md](UPSTREAM.md). Code references are to `crates/core/src/reconcile.rs`, `crates/core/src/sync.rs` and
`crates/native/src/sync.rs`.

## 1. Goal and behaviour

### Goal

Let a person see every file in the cloud in their CraftCloud folder, while only the files they use take disk space.
A Craft app opens a file that is not on disk yet, the client fetches it, and the app gets the real bytes. Saving
works exactly as in v1: guarded upload, version snapshot, conflict copy.

### Non-goals

- Streaming partial content to apps as their main mode of reading (a file is fetched whole before or while it is
  read; ranged reads are an optimisation, not a feature).
- Offline editing of files that were never downloaded.
- Block-level or delta sync of large files.
- Replacing v1. A full sync folder stays the default and the fallback on every platform.
- Sharing or collaboration features in the file manager.

### What the person sees

| State | Meaning | Explorer / Finder / file manager |
|---|---|---|
| Online only | Listed, not on disk. Takes no space beyond metadata | Cloud icon |
| Available | Downloaded on use; may be freed again when space is needed | Green outline tick |
| Always keep on this device | Pinned: downloaded now and never freed | Solid green tick |
| Syncing | Upload or download in progress | Arrows |
| Error / Conflict | As in v1 | Red cross / v1 conflict copy |

Actions, in the file manager context menu where the OS offers one, and in the client's Folders tab for whole
folders:

- **Always keep on this device**: pins the item (and, for a folder, everything below it now and later). The client
  downloads it in the background.
- **Free up space**: turns a downloaded, in-sync file back into a placeholder. Refused, with a reason, for a file
  that has unsynced changes, is open, or is a conflict copy waiting for a decision.
- **Per folder default** in the Folders tab: "Download everything" (v1 behaviour) or "Online only".

Opening a placeholder:

- The app's `open()` or first read waits while the client downloads. A progress notice appears after about a
  second (Windows shows its own; on Linux and macOS the client's tray shows it).
- A Craft app sees an ordinary file. It never has to know about placeholders.
- If the download fails (offline, server error, signed out), the open fails with an I/O error the app can show
  ("The file is not available offline"). The client logs it in Activity.

Offline:

- Downloaded and pinned files work as in v1. Changes queue and upload when the server is back.
- Placeholders cannot be opened. Listing, renaming and deleting them still works locally and is replayed later,
  with the same guards as v1 (a delete is guarded by the last known ETag).
- New files are always created in full and uploaded later.

## 2. Platform mechanisms

Each OS has one mechanism that fits. They differ a lot in who owns the local folder.

| | Windows | Linux | macOS |
|---|---|---|---|
| API | Cloud Files API (cfapi) | FUSE (`/dev/fuse`) | File Provider (`NSFileProviderReplicatedExtension`) |
| Who owns the folder | NTFS; the client fills in placeholders | The client's FUSE server | The OS; the folder lives under `~/Library/CloudStorage/` |
| Hydration trigger | Kernel callback `FETCH_DATA` on first read | `read()` (or `open()`) on the mount | `fetchContents(for:version:request:completionHandler:)` |
| Status icons | Free, from placeholder state | None in the file manager (tray and client only) | Free, from item state |
| Signing needed | No (helps SmartScreen) | No | **Yes**: app extension, entitlements, notarization |
| Rust | `windows` crate, or `cloud-filter` | `fuser` | `objc2-file-provider`, or a Swift extension |

### 2.1 Windows: Cloud Files API

**API.** Win32 `cfapi.h` (Windows 10 1709 and later; target the later builds that the OneDrive client needs, to be
verified per feature):

- Register the sync root with `StorageProviderSyncRootManager.Register` (WinRT, `Windows.Storage.Provider`). This
  gives the navigation pane entry, the status icons and the "Always keep on this device" / "Free up space" menu.
  `CfRegisterSyncRoot` is the lower-level call; the WinRT one is what the shell integration reads.
- `CfConnectSyncRoot` with a callback table: `CF_CALLBACK_TYPE_FETCH_DATA`, `FETCH_PLACEHOLDERS` (for partial
  population), `CANCEL_FETCH_DATA`, `NOTIFY_DELETE`, `NOTIFY_RENAME`, `NOTIFY_DEHYDRATE`.
- `CfCreatePlaceholders` creates entries with size, times and a `FileIdentity` blob (up to 4 KB, ours: the cloud
  path or id plus the ETag).
- `CfUpdatePlaceholder` changes metadata after a remote change; `CfConvertToPlaceholder` turns an ordinary file into
  one; `CfSetInSyncState` drives the in-sync icon.
- `CfHydratePlaceholder` / `CfDehydratePlaceholder` hydrate or free explicitly; `CfSetPinState` with
  `CF_PIN_STATE_PINNED`, `UNPINNED`, `UNSPECIFIED`, `EXCLUDED`, `INHERIT`.

**Hydration.** The first read of a placeholder blocks in the kernel and fires `FETCH_DATA` with a byte range. The
client answers with `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA)` in chunks and reports progress with
`CfReportProviderProgress`. The hydration policy (`CF_HYDRATION_POLICY_FULL` vs `PROGRESSIVE`) decides whether the
app waits for the whole file or only its range. Start with `FULL`; it needs no Range support on the server.

**Dehydration.** `CfDehydratePlaceholder`, from the client or the shell menu. The OS also dehydrates unpinned files
by itself through Storage Sense. It fails on files that are open or not in sync, which is the behaviour we want.

**Rust.** The `windows` crate (Microsoft's generated bindings; features `Win32_Storage_CloudFilters` and
`Storage_Provider`) covers everything; it is mature but raw and `unsafe`. The `cloud-filter` crate is a community
safe wrapper over cfapi; its maintenance state and API coverage must be checked before depending on it. Plan: use
`windows` directly behind a small module in `crates/native`, and borrow ideas from `cloud-filter`.

**Packaging.** No admin rights. Registration needs a stable app identity and an icon resource; an unpackaged app can
register, but some shell features behave better from an MSIX package. The sync root must be on NTFS (not FAT, exFAT
or a network share).

**Pitfalls.**

- An app that saves by writing a temp file and renaming it over the original **replaces the placeholder with an
  ordinary file**. After uploading, the client must call `CfConvertToPlaceholder`, or the file silently stays
  "always local" and loses its icon.
- Placeholders are reparse points (`IO_REPARSE_TAG_CLOUD_*`). How Rust's `std::fs::metadata`, `symlink_metadata`
  and `FileType::is_file` treat them, and whether `read_dir` triggers hydration, must be verified; v1's
  `local_meta` uses `symlink_metadata`.
- Scanning attributes must not open files: use `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, `FILE_ATTRIBUTE_PINNED` and
  `FILE_ATTRIBUTE_UNPINNED` from directory listings.
- Antivirus and the indexer may read files and trigger hydration (see section 5).
- Callbacks run on OS threads with timeouts (about 60 s per request, renewable by reporting progress). A slow server
  must keep reporting progress or the app gets an error.
- Whether cfapi works on GitHub's Windows Server runners is unknown; verify before promising tests there.

### 2.2 Linux: FUSE

**API.** The FUSE kernel protocol over `/dev/fuse`, mounted unprivileged with `fusermount3` (package `fuse3`).

**Layout options.**

| Option | How | For | Against |
|---|---|---|---|
| A. Mount at the sync folder | The FUSE server is the CraftCloud folder; real bytes live in a cache dir (`$XDG_CACHE_HOME/craftcloud/<account>/`) | One folder, same path as v1 | If the client is not running, the folder is empty |
| B. Separate mount | v1 folder stays; a second "CraftCloud (online)" mount shows the rest | v1 untouched | Two places for the same files; confusing |
| C. No FUSE: stub file + xattr | Sparse file of the right size plus `user.craftcloud.state` | No dependency | Apps read zeros (see below) |

Recommended: **A**, a FUSE view over a local cache directory. The state and cache move out of the sync folder
(section 3).

**Hydration.** `lookup`, `getattr` and `readdir` answer from the state, never from the network. `open` returns at
once. `read` serves bytes from the cache file; if the file is not cached, it blocks while the hydration queue
fetches it (whole file first; ranged fetches later, once Range is verified). Blocking in `open` is simpler but
freezes file managers that open files to sniff types, so prefer `read`. `open` with `O_TRUNC` needs no download.
`mmap` arrives as page reads through `read`, so it works without Range support only if the file is fully hydrated
first.

**Dehydration.** Delete the cache file, keep the entry. Only when the file is closed, in sync and not pinned.

**Writes.** Writes go to the cache file. On `release` (last close) the file is marked dirty and the engine uploads
it after the settle time, as in v1. The client sees every write, so v1's `notify` watcher is **replaced** for this
folder, not fixed.

**Rust.** `fuser` is a pure-Rust implementation of the FUSE protocol (with optional libfuse linking for mounting),
widely used and maintained; verify the current version and its support for newer kernel features such as
passthrough before relying on them.

**Pitfalls.**

- **Inode/path mapping:** FUSE speaks inodes. Keep a table inode → cloud path in memory, stable for the mount's
  life. Renames update the table; `forget` counts must be honoured.
- **Rename semantics:** `rename` over an existing name must be atomic (it is how apps save). `RENAME_NOREPLACE`
  and `RENAME_EXCHANGE` flags must be handled or refused with `EINVAL`. Hard links and symlinks: refuse.
- **inotify:** changes made through the mount fire inotify for watchers, but changes the client applies from the
  server do not reach apps watching the folder; `fuser`'s invalidation notifications only drop kernel caches. Apps
  that watch files may not notice remote updates.
- **Mount lifetime:** a crash leaves a dead mount ("Transport endpoint is not connected"); the client must
  `fusermount3 -u` at start. If the mount is missing, the sync folder looks empty: this must never read as "all
  files deleted" (section 3).
- **Packaging:** needs `fuse3` and `/dev/fuse`. Flatpak has no portal for this: a Flatpak build cannot mount FUSE
  without broad device access and a host `fusermount3`. The current Linux package (`packaging/linux/install.sh`)
  is not sandboxed, so it is fine; a future Flatpak would have to fall back to the full sync folder.
- **Option C limits:** without FUSE there is nothing to intercept an open. Apps read zeros from a sparse stub and
  may save them back. The only kernel hook is fanotify permission and pre-content events (`FAN_OPEN_PERM`, and
  the newer pre-content events meant for this use; verify the kernel version), which need `CAP_SYS_ADMIN`. Not
  acceptable for a desktop client. A safer non-FUSE fallback is a stub with a different name
  (`report.docx.cloud`) that the client opens via a MIME handler, at the cost of apps not seeing the real name.

### 2.3 macOS: File Provider

**API.** `NSFileProviderReplicatedExtension` (macOS 11 and later; target macOS 12 or later, where it is mature;
verify). The extension is a separate, sandboxed process:

- `NSFileProviderManager.add(_ domain:)` registers a domain; the folder appears under `~/Library/CloudStorage/`,
  **not** at the folder the person chose in v1. This is a visible change.
- `enumerator(for:)` returns an `NSFileProviderEnumerator`: `enumerateItems` lists a folder, `enumerateChanges`
  returns changes since an `NSFileProviderSyncAnchor`. The **working set** (`.workingSet`) enumerator reports
  changes for everything the system tracks.
- `fetchContents(for:version:request:completionHandler:)` hydrates; `createItem`, `modifyItem`, `deleteItem` receive
  local changes.
- `NSFileProviderManager.signalEnumerator(for:)` tells the system to ask for changes; `evictItem(identifier:)`
  dehydrates. Pinning uses the item's content policy (newer macOS; verify which version).

**Item identifiers.** Every item needs a stable `NSFileProviderItemIdentifier`. OxiCloud's WebDAV has no file id
(`oc:fileid` → 404), so either:

- use the **path** as the identifier: a rename becomes delete plus create, the system loses its state for the item,
  open documents may be closed, and a large folder rename re-creates every child; or
- use OxiCloud's **REST ids** (section 4), if they prove stable across rename and move.

**Rust.** `objc2-file-provider` is generated from the SDK headers by the objc2 project; it exposes the classes, but
writing an extension (subclassing, the `.appex` bundle, `Info.plist` with `NSExtensionPointIdentifier
com.apple.fileprovider-nonui`) from Cargo is uncharted. Plan: a **small Swift extension** that links the Rust core
as a C-ABI static library (`crates/fileprovider-ffi`), and the main app talks to it through the shared app group
container and `NSFileProviderManager` calls.

**Packaging. Blocking for v1:** the extension only loads when the app and the extension are code-signed with a
Developer ID, carry the app group entitlement, and are notarized. v1 desktop builds are unsigned. Files on demand
on macOS cannot ship until signing exists.

**Pitfalls.** The extension has strict memory limits and can be killed at any time; all state must be on disk.
Debugging needs a signed build on a real Mac. Enumeration is pull-based: the system decides when to ask.

## 3. What the engine and state need

### State schema

`Saved` in `crates/native/src/sync.rs` has no version field today. Add one, with serde defaults so every existing
`state.json` loads unchanged:

```rust
struct Saved { #[serde(default)] version: u32, /* 0 = v1 */ ... }

struct Known {
    etag: String, size: u64, mtime: i64,
    #[serde(default)] presence: Presence,   // Full (default), Placeholder
    #[serde(default)] pin: Pin,             // Unspecified (default), AlwaysKeep, OnlineOnly
    #[serde(default)] last_access: Option<i64>,
}
```

- `Presence::Full` is v1's meaning, so a v1 state is a valid files-on-demand state.
- `Pin` on folders lives in a separate map, inherited by children.
- **Move state and cache out of the sync root.** Today `state.json` lives in `<local>/.craftcloud/`. A FUSE mount at
  the sync folder hides it, cfapi would show it in Explorer, and File Provider gives no folder at all. Move it to
  the platform's data dir (`$XDG_DATA_HOME`, `%LOCALAPPDATA%`, the app group container), keyed by account and
  folder. The cloud's `.craftcloud/` (versions) is unaffected and is still never synced or shown.

### Reconcile

`local_changed` compares size and mtime. A placeholder has no honest local mtime, and its reported size differs by
platform (cfapi reports the logical size, a sparse stub reports it too, FUSE reports what we tell it). So
`reconcile` must branch on presence **before** calling `local_changed`. `LocalFile` gains `placeholder: bool`,
filled by the platform scan (cfapi attributes, the FUSE table; File Provider has no local scan, its events arrive
as `createItem`/`modifyItem`/`deleteItem`).

New actions: `CreatePlaceholder(path)`, `UpdatePlaceholder { path, etag, size }`, `Hydrate(path)`,
`Dehydrate(path)`.

Rows for a placeholder (known as `Placeholder`, local is a placeholder):

| Known | Local | Remote | Action |
|---|---|---|---|
| placeholder | placeholder | same ETag | none |
| placeholder | placeholder | new ETag | `UpdatePlaceholder` (no download), or `Download` if pinned |
| placeholder | placeholder | gone | `DeleteLocal` (removes the placeholder; no bytes lost) |
| placeholder | gone | same ETag | `DeleteRemote` with `If-Match` (a deleted placeholder is a real delete) |
| placeholder | gone | new ETag | `CreatePlaceholder` (the cloud change wins, as in v1) |
| placeholder | full file (hydrated by the OS) | same ETag | record `Full`; no transfer |
| placeholder | full file, changed | new ETag | `Conflict` (as v1) |
| none | none | present, folder is online only | `CreatePlaceholder` instead of `Download` |

A placeholder can never cause an upload or a conflict: it has no local bytes. Each row is one unit test in
`crates/core/src/reconcile/tests.rs`.

### Hydration queue

- Hydration requests come from the OS (cfapi callback, FUSE `read`, `fetchContents`) and must not wait for the 30 s
  cycle. A separate worker pool (start with 2) shares the session and tokens and **has priority** over background
  transfers; the background cycle pauses its downloads while a hydration is waiting.
- A hydration downloads with `If-Match: <known etag>`. On 412 the file changed in the cloud: update the placeholder
  and fetch the new version (safe, there are no local changes).
- The cycle skips paths that are being hydrated, and records the result through the same state lock.

### Partial downloads and resume

v1 downloads to `.<name>.craftcloud-part` and starts over on failure. For large files, resume with
`Range: bytes=<n>-` plus `If-Range: <etag>`.

- OxiCloud's OpenAPI (fixture `openapi.json`) documents a `206 Partial Content` answer on REST
  `GET /api/files/{id}`.
- Range on **WebDAV** `GET`, which the engine uses today, is not documented. **It must be verified** against the
  pinned server before resume or progressive hydration is built.
- OxiCloud also has a chunked delta protocol (`/api/files/delta/negotiate`, `chunks`, `commit`, `download`;
  FastCDC and BLAKE3). It could resume and dedupe, but it is a custom protocol; prefer plain Range.

### Cache eviction

- Only files that are `Full`, in sync (ETag matches the last listing), closed, not pinned and not conflict copies.
- Least recently used first (`last_access`), until under a size limit set in Settings (default: none; "free up
  space" is manual until there is a reason).
- Windows: let Storage Sense do it and only honour `NOTIFY_DEHYDRATE`. macOS: the system evicts on its own.

### Versions and conflict copies

- Version snapshots are a server-side `COPY` to `.craftcloud/versions/<path>/<timestamp>` before an overwrite. They
  need no local bytes, so placeholders change nothing. Keep 5, keyed by path, as in v1.
- A local edit always hydrates first, so conflicts only happen on full files and keep v1's naming:
  `name (conflict YYYY-MM-DD HH.MM Device).ext`. The conflict copy is created full and uploaded.
- New on both sides: v1 downloads and compares bytes. With online-only folders, the local file is full by
  definition (the person made it), so the comparison is unchanged.
- Restoring a version from the web frontend changes the ETag; the client updates the placeholder.

### Safety rails

- **Dehydration is never deletion.** It changes `presence`, never removes a state entry, and never counts toward the
  mass-delete limit (`(known files / 2).max(5)`). Placeholders count as present in the local tree.
- **A missing mount is not an empty folder.** v1 pauses when the local folder is missing. With FUSE, an unmounted
  sync folder exists but is empty: check that the folder is our mount (`statfs` type and a marker) before scanning,
  and pause otherwise. On Windows, check the sync root is still registered and connected.
- The client reports "ready for offline" only once every pinned file is downloaded.

### Status

`SyncState` stays global. A per-file status is added for the file manager, the client and the tray:
`Synced`, `Syncing`, `Error`, `Conflict`, plus **`OnlineOnly`** and `AlwaysKeep`. `SyncProgress` gains the number of
hydrations waiting and running, so the tray can show "Downloading report.docx to open it".

## 4. Server-side needs and gaps

| Need | OxiCloud 0.10.0 | Why it matters | Workaround |
|---|---|---|---|
| Stable file id | WebDAV: none (`oc:fileid` → 404). REST: files and folders have ids (`/api/files/{id}`, `/api/folders/{id}/resources`) | File Provider identifiers; rename without losing history; cfapi `FileIdentity` | Path ids; or a path↔id map from REST, **if** ids survive rename and move (to verify) |
| Change feed / sync token | None (no WebDAV `sync-collection` REPORT, RFC 6578; no REST "changes since") | Today every folder is listed every cycle; with online-only folders the tree can be much larger | List everything, 4 folders at a time |
| Reliable folder ETags | Not reliable (UPSTREAM C8) | Would let the scan skip unchanged subtrees | None |
| Range on WebDAV `GET` | Unknown; REST `GET /api/files/{id}` documents 206 | Resume, progressive hydration, `mmap` | Whole-file download; or REST for downloads |
| Server-side versions | None (UPSTREAM C3) | Client snapshots cost a `COPY` and a listing per overwrite | `.craftcloud/versions/` |
| Content hash in listings | Not in PROPFIND; REST has `/api/files/{id}/manifest` and `/api/files/by-hash` | Compare without downloading (new on both sides) | Download and compare |
| Thumbnails | REST `GET /api/files/{id}/thumbnail/{size}` | Placeholders can show previews without hydrating | n/a |

Worth proposing upstream (drafts go into UPSTREAM.md first; **ask before filing**, per the project rule):

1. A change feed: WebDAV `sync-collection` (RFC 6578) or a REST cursor endpoint. The biggest win.
2. A file id over WebDAV (`oc:fileid`, or `DAV:resource-id` from RFC 5842), equal to the REST id and stable across
   rename and move.
3. The folder ETag fix (C8), already drafted.
4. Range on WebDAV `GET`, if verification shows it is missing.

## 5. Risks

| Risk | What could happen | Mitigation |
|---|---|---|
| Dehydrating a file with unsynced changes | Edit lost | Refuse unless `Full`, in sync and closed; OS APIs refuse too; unit tests |
| Unmounted FUSE folder read as empty | Mass delete in the cloud | Mount check before every scan; mass-delete pause stays |
| Temp file + rename saves (Craft apps write `.craft_tmp` then rename) | cfapi: file loses placeholder status; FUSE: rename must be atomic | Convert back after upload; FUSE rename tests; `ignored()` already skips `.craft_tmp` |
| Thumbnailers, indexers, antivirus, backup tools reading the folder | Mass hydration, full disk, server load | Per-process hydration rate limit; cfapi gives the requesting process id; consider refusing known indexers on FUSE |
| Craft apps that scan a whole folder (project media, asset browsers) | Hydrate a project on open | Accept; suggest "Always keep" for project folders |
| Offline with placeholders | Files a person expected are missing | Clear "online only" icon; pin folders before travel; a warning when pinned files are not all down |
| Path-based ids on macOS | Renames drop item state, close open documents | Wait for stable ids before shipping macOS |
| Signing and notarization | macOS feature cannot load at all | Blocked until signed builds exist |
| Callback timeouts | Slow server fails the app's open | Progress reporting; whole-file fetch with a clear error |
| Testing | Needs real Windows and macOS, kernel features, GUI shells | See below |

Testing difficulty, honestly: CI runs on Linux; Windows and macOS build only on release tags and manual runs (a
choice made while the repository was private, where those minutes cost more). A GitHub Windows runner cannot easily
run the Linux server image, so Windows live tests need a server reachable from it, or a local Windows VM against a
throwaway server. FUSE tests need `/dev/fuse` in the runner (available on GitHub's Ubuntu runners; to verify). macOS tests need a signed build.

## 6. Staged plan

Recommended order: **Stage 0 (prerequisites) → Stage 1 core state machine → Stage 2 Windows cfapi → Stage 3 Linux
FUSE → Stage 4 macOS File Provider.**

Why Windows before Linux: cfapi keeps placeholders as real files on NTFS, so v1's engine shape (scan the local
folder, reconcile, act) survives; the OS does the hard parts (blocking the app, icons, the pin menu, Storage
Sense); and it matches the OneDrive model people know. FUSE inverts the engine (the client becomes a file system)
and has the Flatpak gap. The counterweight: development and CI are on Linux, so Linux stages are cheaper to test.
If that cost decides it, swap Stages 2 and 3; the tiebreaker is where the Craft app users are, which is an open
question. macOS is last because it is blocked on signing whatever the order.

### Stage 0: prerequisites

- Add `version` to `Saved`, with serde defaults; v1 state loads as version 0.
- Move `state.json` and the download temp files out of the sync folder into the platform data dir.
- Verify on the pinned server, recorded as fixtures in `crates/core/tests/fixtures/oxicloud-0.10.0/`:
  - WebDAV `GET` with `Range` and `If-Range`.
  - Whether REST ids survive `PUT /api/files/{id}/rename`, `/move`, and a WebDAV `MOVE`; whether REST ETags equal
    WebDAV ETags.
- Decide: path ids or REST ids and record it in this document.

Exit: state migration test passes; the fixtures exist; the id decision is written down.

### Stage 1: the state machine in `crates/core`

- `Presence`, `Pin`, the new actions, the placeholder rows of the table, folder pin inheritance, eviction choice.
- A `Presence` trait in `crates/native` with a fake implementation (marks files as placeholders in the state and
  deletes the bytes), so the whole flow runs against a real server with no OS integration.

Exit: one unit test per table row; mass-delete tests prove dehydration never counts; a live test in the style of
`crates/native/tests/live.rs` ("Laptop" and "Studio" against a throwaway server from `scripts/smoke-test`): Studio
has a folder online only, Laptop edits a file, Studio's placeholder updates without a download, Studio hydrates it
and gets the new bytes.

### Stage 2: Windows cfapi

- Sync root registration, placeholder creation from the state, `FETCH_DATA`, rename and delete notifications,
  pin states, `CfConvertToPlaceholder` after temp-file saves, in-sync state for icons.
- Opt-in per folder in the Folders tab, marked experimental.

Exit: on a real Windows machine, a Craft app opens a placeholder and saves it back; "Free up space" and "Always keep
on this device" work from Explorer; unplugging the network during a hydration fails the open cleanly; a temp-file
save keeps the file a placeholder. Manual test script in the repo; the cfapi wrapper has unit tests for its pure
parts.

### Stage 3: Linux FUSE

- `fuser` mount at the sync folder over a cache dir; inode table; `read`-time hydration; atomic rename; release
  triggers upload; mount check and stale-mount cleanup.
- Falls back to the full sync folder when `/dev/fuse` or `fusermount3` is missing.

Exit: live tests on Linux CI if `/dev/fuse` is available there: open a placeholder with `cat`, save with temp file
plus rename, kill the client and check the next start cleans the mount and does not delete anything.

### Stage 4: macOS File Provider

- Needs signed and notarized builds first. Then the Swift extension over the Rust core, path or REST ids per Stage 0,
  enumerator and working set from the state, `fetchContents`, eviction and pinning.

Exit: on a real Mac, a Craft app opens and saves through `~/Library/CloudStorage/`; a rename keeps the item (only if
stable ids exist); the extension survives being killed mid-download.

### Stage 5: server improvements (any time)

Use a change feed, stable ids and Range when upstream ships them; remove the full listing per cycle.

## 7. Open questions

1. Does OxiCloud 0.10.0 answer WebDAV `GET` with `Range` (206) and honour `If-Range`?
2. Are REST file ids stable across rename and move, including moves made over WebDAV? Is the REST ETag the WebDAV
   ETag?
3. Which OS do most Craft app users run? It settles Stage 2 versus Stage 3.
4. Should online only be the default for new folders, or only an opt-in per folder?
5. Is a size limit for the cache needed in the first release, or is "Free up space" by hand enough?
6. On Linux, is moving the sync folder under a FUSE mount acceptable for people who also back up that folder (a
   backup tool would hydrate everything)?
7. On macOS, is the move to `~/Library/CloudStorage/` acceptable, and how do existing v1 folders migrate?
8. Should hydration be refused or throttled for known indexers and thumbnailers, and how is a process identified
   on each OS?
9. Is MSIX packaging worth it on Windows for better shell integration, given that builds are unsigned in v1?
