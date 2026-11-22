# CraftCloud: architecture

## Overview

```
desktop client ─┐
browser ────────┴─► :8086  Caddy (inside the CraftCloud container)
                      ├─ /craftcloud/config.json, /craftcloud/api/*  ─► 127.0.0.1:8088  craftcloud-admin
                      ├─ /craftcloud/*            static: apps.json, icons, the shim
                      ├─ /apps/<app>/*            the Craft web builds (-apps image only), each request
                      │                           checked first by craftcloud-admin (forward_auth)
                      ├─ /api, /webdav, …         ─► 127.0.0.1:8087  OxiCloud (upstream image, unmodified)
                      └─ everything else          CraftCloud's web frontend (egui, WebAssembly)
PostgreSQL (a second container, or the host's)
```

## The server image

- **OxiCloud runs unmodified.** The image starts `FROM diocrafts/oxicloud:<pinned tag>`. CraftCloud compiles none of
  OxiCloud and patches none of it; gaps go upstream as pull requests ([UPSTREAM.md](UPSTREAM.md)).
- **Caddy is the front,** inside the same container. It serves the web frontend and the Craft web apps with their
  own headers (same-origin framing, precompressed `br`/`gzip`), sends the API, WebDAV and device sign-in to OxiCloud,
  and blocks OxiCloud's own pages and the routes CraftCloud doesn't use.
- **craftcloud-admin** is a small Rust service (`apps/craftcloud-admin`, a static musl binary) on `127.0.0.1:8088`.
  It serves `/craftcloud/config.json` and the server settings API (owners and admins only, CSRF-checked), and
  answers Caddy's `forward_auth` gate before every `/apps/*` file.
- **Settings** live in `/app/storage/.craftcloud/settings.env` on the data volume. `CRAFTCLOUD_WEB_APPS` in `.env`
  only seeds the first start; after that the file decides.
- **Two images:** `craftcloud:<ver>` (slim) and `craftcloud:<ver>-apps` (with all Craft web builds). The final
  stage only copies files, so a multi-arch build needs no emulation; the Rust parts are cross-compiled per
  architecture.
- **OxiCloud's own apps** (Music, Photos, the Markdown editor, viewers, WOPI office editors, CalDAV/CardDAV) are off
  by default; `CRAFTCLOUD_NATIVE_APPS` turns some back on.

## The Rust workspace

Laid out like the ArtCraft Launcher:

| Path | What it is |
|---|---|
| `crates/core` | Platform-free logic: the OxiCloud API (auth, device sign-in, WebDAV, user admin), server URLs, the sync decision table (`reconcile`), the app catalog. Tested against recorded server answers in `tests/fixtures/` |
| `crates/native` | Desktop-only parts: HTTP, the keychain, settings, start at login, the sync engine and its storage layer (`storage`) |
| `crates/ui-egui` | Every screen, for both the desktop client and the web frontend, drawn with egui in the launcher's look |
| `apps/craftcloud` | The desktop client (window, tray) and the command line ([CLI.md](CLI.md)) |
| `apps/craftcloud-cli` | `craftcloud.com` on Windows: a console program that runs `craftcloud.exe` (a GUI program) for a terminal and passes its output through ([CLI.md › Windows](CLI.md#windows)) |
| `apps/craftcloud-web` | The web frontend, built with Trunk |
| `apps/craftcloud-admin` | The image's settings service |
| `xtask` | `cargo xtask ci`: fmt, clippy, tests and the wasm build. `fetch-apps` downloads, checks (sha256), unpacks and precompresses the Craft web builds for the `-apps` image; `bump-apps` updates their pins in `apps.toml`; `icons` renders the packaging icons from the logo |

## Sync

The desktop client keeps chosen server folders in a local folder, two ways.

- **Sign-in** uses OxiCloud's OAuth 2.0 device flow (RFC 8628): the client shows a code, the person approves it on
  the server's `/device` page, and the client stores the tokens in the system keychain.
- **Each cycle** lists every server folder over WebDAV (one level at a time, four at once; folder ETags are not
  reliable, so none are skipped) and the local folder, and `core::reconcile` decides per file: upload, download,
  delete, or conflict. `core::reconcile::restrict` then cuts that plan to the mode the server's capabilities allow
  (`core::storage::mode_for`): two-way (OxiCloud; nothing is cut), download only (nothing is written, moved or
  deleted in the cloud; a file deleted there goes into `.craftcloud/trash/` in the local folder), or refused
  (nothing). See [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 7.3 and § 9.
- **Writes are guarded:** a new file is created with `If-None-Match: *`, an overwrite carries `If-Match: <etag>`, so a
  change made elsewhere in between is never lost.
- **Before an overwrite**, the old version is copied to `.craftcloud/versions/<path>/<time>`; the last 5 are kept, and
  the same bytes are never kept twice. A rename or move (of a file or a folder) takes the versions along.
- **Renames and moves** are moves on the other side, never a delete plus an upload: a file is paired by size and
  time (here) or by ETag (in the cloud; a `MOVE` keeps it), and a whole folder whose synced files are all in a new
  folder at the same relative paths is one WebDAV `MOVE` (it keeps its id, shares and stars) or one local rename.
  Changes made meanwhile on the other side move along and sync afterwards; a partial match syncs file by file. A move the
  cloud can't do right now (a 5xx, 423 Locked, no answer) is asked again next pass, and until then nothing under the
  old or the new name syncs; only a move refused for what it is syncs file by file. A move that keeps failing is asked for
  again for as long as it takes (after the third try with longer waits: 2 minutes, then 5), and the window, the tray
  and a notification say so from the third try on.
- **A deleted folder** goes to the cloud's trash as one item (so restoring it brings it back whole), when everything
  in it is unchanged in the cloud and nothing new is there; otherwise its files go one by one.
- **Conflicts** keep both: `name (conflict YYYY-MM-DD HH.MM Device).ext`. A file on one side and a folder of the same
  name on the other keep the cloud's, and the local one becomes the conflict copy. A synced folder replaced by a file
  of its name (or a synced file by a folder) on one side, while the other side is unchanged since the last sync, is a
  replacement, not a conflict: the old one is removed there (a folder as one trash item), then the new one follows.
- **Names**: one the cloud can't take (a backslash, a control character) is never sent and is listed once as not
  synced; one a Windows computer can't have (`<>:"|?*`, a trailing dot or space, CON, NUL, COM1 …, also with an
  extension) syncs with a warning on Linux and macOS, and is left alone on Windows (never downloaded, uploaded or
  taken as deleted; listed once as not synced). One rule decides both (`core::names::windows_trouble`, through
  `names::Platform`). On Windows, cloud names that differ only in case are held, all but the one the computer has.
  An upload the cloud refuses (a 4xx) waits until the file changes.
- **Never synced**: lock, swap and temporary files of apps and editors (`.~lock.*#`, `~$*`, `*.swp`, `.#*`, `#*#`, `*~`, `*.tmp`, `.DS_Store`,
  `Thumbs.db`, `desktop.ini`, …; the list is `core::names::ignored`). Symbolic links are not followed and are
  listed once as not synced. File permissions (the executable bit) are not synced: OxiCloud keeps none.
- **An interrupted download** resumes where it stopped (HTTP `Range`) when the cloud still has the same version.
  Any other failed download removes its part file, and part files older than a day are cleaned up.
- **Mass deletes wait** for a decision (delete them, or bring the files back) while everything else syncs.
- **A missing sync folder** (moved, deleted, an unplugged drive) holds the sync; it is never made again on its own,
  only when the person chooses "Download everything again" (or sets the client up again).
- **The storage layer** ([WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 7): the engine (`crates/native/src/sync.rs`)
  talks to the server only through the `RemoteStorage` trait (`crates/native/src/storage.rs`), which has no
  unguarded write: a create, an overwrite, a move and a trash each take the guard they must hold, and a permanent
  delete exists only for an `OwnPath` inside `.craftcloud/`. Two drivers: OxiCloud's
  (`storage/oxicloud.rs`: WebDAV for bytes and listings, REST for the trash, its ids and the quota) and the generic
  WebDAV driver (`storage/webdav.rs`, behind `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` until 0.3.0: any other WebDAV server
  with an app password, a trash by `MOVE` into `.craftcloud/trash/<time>-<device>/<path>`, versions by `COPY` into
  `.craftcloud/versions/`, hrefs read against its base so one outside it fails the listing). What a server
  guarantees is a `Capabilities` (`crates/core/src/storage.rs`, pure data): OxiCloud's filled in by hand, another
  server's measured by the check on connect (`storage/probe.rs`, `storage-check.json` in the settings folder) when
  the driver connects, which the engine does on its sync thread. The
  requests the engine sends are recorded per scenario (`crates/native/src/sync/golden/`) and checked on every test
  run, along with the rule that every write carries its guard; the engine's tests run over both drivers.

## Apps in the browser

Optional and off by default (`-apps` image plus the setting). The Craft web builds sit in a same-origin iframe and
talk to the host through `craft-web-host` (`ready`, `open`, `save`, `dirty`, `failed`). For apps without that bridge,
the image injects `craftcloud-shim.js`, which speaks the same protocol on the app's behalf; those apps are marked
experimental. `apps.toml` pins each app's version and checksum and sets its bridge and file types; `cargo xtask bump-apps`
moves the pins to each app's latest release, and `cargo xtask fetch-apps` builds the image's app tree from them.

**Status:** the image serves the web builds and gates each file per app; the page that frames an app and the shim
are not built yet ([ROADMAP.md](ROADMAP.md)).

## Branding

CraftCloud has its own name and mark (the mackerel). It follows the ArtCraft Launcher's look (MIT OR Apache-2.0) but
uses no ArtCraft logo or mark, and never puts "ArtCraft" in its product name, header or icon.
