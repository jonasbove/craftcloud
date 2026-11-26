# CraftCloud roadmap

**Stage: early alpha** · version 0.2.0 (share links and version history on the web, the Windows fixes, server hardening)

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-26 · **Change:** minor (the desktop client says when a new version is out; version 0.2.0; the web header's Apps menu for the Craft web apps, for 0.3.0; M9 moves to
> 0.3.0; M9 pull requests 1 to 6, the sync engine behind a storage trait, sync modes, the check on connect with `craftcloud server check`, the generic WebDAV driver, the client trash's retention, the Nextcloud profile, the trash and version commands and the runtime watch, all behind the `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` opt-in); on Linux the tray counts as up only while a tray
> host is on the bus, so closing the window on stock GNOME quits instead of leaving an invisible client; a start at
> login stays in the tray on Windows and X11, and opens the window where there is no tray; an offline desktop window waits before asking who is signed in again; the
> release checklist and the Release workflow check the Craft web app pins; an idle window uses next to no CPU; trace
> logs leave out HTTP bodies and headers; a cloud file listed without an ETag is never trashed; the server scripts
> need only sh and curl, no Python)

CraftCloud is a self-hosted cloud for the Craft apps: the unmodified OxiCloud server in a Docker image, CraftCloud's
own web frontend (Rust, egui, WebAssembly), a desktop sync client with a command line, and a small admin service.
This page is the summary: where each part stands, the milestones, what comes next and what has landed.
[STATUS.md](STATUS.md) says what works today, how it was checked, and the known limits.

**Why early alpha:** the server image, the sync client, the web frontend and the command line all work end to end
against OxiCloud 0.10.0, and the sync has been through a round of testers' reports (T-001 to T-064). 0.10.0 is the first
release, its desktop packages are new, and the desktop builds are unsigned. Keep a
backup of anything you sync with it.

## By area

| Area | State | Doc |
|---|---|---|
| Server image (OxiCloud unmodified, Caddy, craftcloud-admin, slim and `-apps`) | done; built from source only | [ARCHITECTURE.md](ARCHITECTURE.md) |
| First-run setup (browser and terminal) | done | [README](../README.md#run-the-server) |
| Desktop sign-in (device flow, keychain) | done | [ARCHITECTURE.md › Sync](ARCHITECTURE.md#sync) |
| Single sign-on (OpenID Connect) | done; tested end to end with Keycloak | [OIDC.md](OIDC.md) |
| Two-way sync folder, versions, conflict copies | done; hardening from testers' reports | [ARCHITECTURE.md › Sync](ARCHITECTURE.md#sync) |
| Desktop client (tray, Activity, Folders, Settings, start at login, says when a new version is out) | done | [README](../README.md#downloads) |
| Web file browser (upload, download, rename, move, delete, trash, versions, details, recent, starred, search) | done | [design-notes/pass-2.md](design-notes/pass-2.md) |
| Share links | done: public share page, CLI, the web's "Share link" sheet and "Your share links" page | [CLI.md › Share links](CLI.md#share-links) |
| Admin (users, server settings, Craft web apps on or off) | done, on the web and in the CLI | [CLI.md](CLI.md#server-administration-owners-and-admins) |
| Command line | done; `save --if-match` and `etag` in listings for cloud backends; on Windows through `craftcloud.com` (untried on Windows); `server check` says what the server guarantees the sync; behind `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`, `login --kind webdav|nextcloud` and the file and sync commands on any other WebDAV server or a Nextcloud (its own trash and versions) | [CLI.md](CLI.md) |
| Craft apps through the sync folder | done (every app) | [CRAFT-INTEGRATION.md § 1](CRAFT-INTEGRATION.md#1-the-sync-folder-works-today-every-app) |
| `craftcloud://` deep links | done on Linux and Windows; macOS needs an Apple Event handler | [CRAFT-INTEGRATION.md § 2](CRAFT-INTEGRATION.md#2-deep-links-craftcloud) |
| Integration contract (`craftcloud apps`, `files`, `save`) | done; desktop-only apps (`web = false`) listed too | [CRAFT-INTEGRATION.md § 3](CRAFT-INTEGRATION.md#3-the-integration-contract-the-craftcloud-command) |
| Craft web apps in the browser | in part: the `-apps` image serves and gates them, and the web header's Apps menu opens the ones switched on in a new tab (0.3.0); opening a cloud file in one and saving it back is not built | [CRAFT-INTEGRATION.md § 6](CRAFT-INTEGRATION.md#6-the-craft-web-apps-in-the-browser) |
| "Open from / Save to CraftCloud" in the launcher and the apps | proposal, upstream: for the apps, a craft-libs `CloudBackend` that runs `craftcloud` (UPSTREAM.md F1); the command-line half (an ETag in `files`, `save --if-match`, a read to stdout) is next | [CRAFT-INTEGRATION.md § 4–5](CRAFT-INTEGRATION.md#4-the-artcraft-launcher-proposal-upstream) |
| Files on demand | design only | [FILES-ON-DEMAND.md](FILES-ON-DEMAND.md) |
| Releases (published image, desktop release, signing) | desktop packaging done, not yet run in CI: the Craft apps' release file set (AppImage, deb, rpm, Flatpak, Arch, tar.gz; MSI and portable zip; universal DMG) from `release.yml` as a draft release, GPG-signed when a key is set, and only after every Linux package passed its install test (`package-test`); no published image, no code signing | [README](../README.md#downloads) |
| Upstream gaps and proposals | drafted, not filed | [UPSTREAM.md](UPSTREAM.md) |

## Milestones

| Milestone | Goal | Where it stands (2026-11-14) |
|---|---|---|
| M0 Server image | OxiCloud unmodified behind Caddy, CraftCloud's settings service, slim and `-apps` images, NixOS module | Done |
| M1 Sync client | Device sign-in, a two-way sync folder with guarded writes, versions and conflict copies, tray | Done; hardening continues from testers' reports |
| M2 Web frontend | Setup, sign-in, device approval, home, the file browser, admin, single sign-on, share links | Done |
| M3 Command line and integration | `craftcloud <command>` for files, sync, sharing, versions and admin; the contract the Craft apps and the launcher can use | Done; the command line prints in Windows terminals too (tried on Windows 11) |
| M4 First release | Published images, a tagged desktop release, release notes | Done: 0.1.0 on 2026-11-14, then 0.1.1 and 0.2.0 |
| M5 Craft web apps in the browser | Open a cloud file in its Craft web app from the web file browser and save it back (`craft-web-host`, the shim for the others) | Serving and gating done; the Apps menu in the web header opens an app in a new tab (0.3.0); the editor page and the shim not built |
| M6 Files on demand | Placeholders that download when opened (Windows, Linux, macOS) | Design only |
| M7 Signed desktop builds | Signed and notarized macOS builds, signed Windows builds | Not started; v1 is unsigned |
| M8 Native NixOS deployment | OxiCloud, Caddy and craftcloud-admin as NixOS services without Docker, with the Craft web apps built by Nix from `apps.toml` | Not started; the NixOS module runs the Docker image today |
| M9 Any WebDAV server as the storage | The desktop client and command line sync with any WebDAV server (Nextcloud, ownCloud, a NAS, `rclone serve webdav` …), not only OxiCloud: a check of the server on connect, and the client's own trash and versions where the server has none | Planned for 0.3.0, on `main` behind the `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` opt-in until the release; the storage trait with OxiCloud as its first driver, the sync modes (two-way, download only, refused) with the plan cut to each, the check on connect (the probe's steps and verdict in `core`, its runner in `native`, `craftcloud server check`) and the generic WebDAV driver (an app-password sign-in, the client's own trash by `MOVE` and versions by `COPY`, the command line's file and sync commands through the trait, the client trash's 30-day retention by the server's clock, the trash and version commands through the trait, the check's re-run schedule and the runtime watch) are in; without the opt-in a person sees only `server check`, which reports CraftCloud's own server's known capabilities. The live suite runs against Nextcloud, ownCloud, OpenCloud, `rclone serve webdav` and Apache `mod_dav` by hand (`CRAFTCLOUD_TEST_KIND`, `tests/other-servers/`), asserting the mode each gets. Nextcloud's profile is in (6: Login Flow v2, its trash and versions after the check's round trips, the checksum check on every write). Next: the ownCloud and OpenCloud profiles (7, 8), the CI job (9), the desktop's Connect sheet (10) |
| M10 No Python in the tree | The operator scripts without Python (sh and curl) and the packaging icons in Rust (0.3.0), then the design build (`design/tools`, FigCraft) as `cargo xtask design` | 0.3.0: scripts and icons in progress; the design build later |

## Current focus

Sync correctness from testers' reports (the T-numbers in the commit log: renames, folder deletes, unreadable
places, names the cloud can't take, interrupted downloads, quota) and the second UI pass over the web frontend
([design-notes/pass-2.md](design-notes/pass-2.md)).

## Upcoming

In the order planned so far; detail in [STATUS.md](STATUS.md#known-limits) and the docs linked.

1. **The AUR packages:** the recipes in `packaging/arch/` (`craftcloud`, `craftcloud-bin`) published to the AUR.
2. **Craft web apps in the browser:** the page that frames an app and talks `craft-web-host`, and the shim for the
   apps without it ([CRAFT-INTEGRATION.md § 6](CRAFT-INTEGRATION.md#6-the-craft-web-apps-in-the-browser)).
3. **macOS deep links:** an Apple Event (`kAEGetURL`) handler, so a `craftcloud://` click opens the file.
4. **Files on demand,** in the stages of [FILES-ON-DEMAND.md § 6](FILES-ON-DEMAND.md#6-staged-plan).
5. **Upstream:** the drafts in [UPSTREAM.md](UPSTREAM.md) and server-side versioning for OxiCloud, filed only after
   they are agreed.
6. **Unmaintained D-Bus crates:** drop the `deny.toml` ignores for zvariant, zvariant_derive and zbus_names
   (RUSTSEC-2026-0370 to 0372) once egui (accesskit) and keyring move to zbus 6.
7. **A native NixOS deployment (M8):** today `nix/module.nix` runs the Docker image, which already holds the Craft
   web apps. A Nix derivation (`nix/apps.nix`) would read `apps.toml` with `builtins.fromTOML`, fetch each pinned
   release zip with `fetchurl` and its sha256, unpack and precompress it, and write `apps.json`: reproducible,
   cached in the Nix store, and no network during the build. It is worth doing together with running OxiCloud,
   Caddy and `craftcloud-admin` as NixOS services, not while the module runs the image, since two ways of
   fetching the apps would drift apart. `scripts/bump-apps` (moving to `cargo xtask` in 0.2.0) stays the way to
   move the pins.
8. **Any WebDAV server as the storage (M9):** the desktop client and the command line sync with a WebDAV server of
   the person's choosing, not only OxiCloud. First the sync engine moves behind a storage layer that says what the
   server guarantees, with OxiCloud as its first driver; then generic WebDAV:
   - **A check on connect.** In `.craftcloud/probe/` the client tries what the sync relies on: create only if absent
     (`If-None-Match: *`), overwrite only if unchanged (`If-Match`, a 412 when stale), an ETag that changes on every
     write and survives a `MOVE`, folder ETags that follow their contents, `Depth: infinity`, and what a `PUT` into a
     missing folder answers.
   - **What the server lacks, the client makes up for.** A WebDAV `DELETE` is taken as permanent, so a delete is a
     `MOVE` into `.craftcloud/trash/`; versions stay in `.craftcloud/versions/`; where folder ETags follow their
     contents, unchanged folders are skipped.
   - **Never a weaker two-way sync.** A server that ignores either guarded write gets a one-way or read-only mode,
     never two-way. Servers differ here, so the check runs against each, not once per product.
   - **Known servers add what they have.** Nextcloud and ownCloud: their own trash, versions and file ids, and
     Nextcloud's sign-in in the browser (Login Flow v2). Elsewhere the person signs in with an app password made on
     the server, never the account's password.
   - Desktop and command line only; the web frontend stays with OxiCloud. It changes how the sync decides, so it
     comes with an "upgrade all devices together" entry in [RELEASE-NOTES.md](RELEASE-NOTES.md).
   - The design, with the decisions taken: [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) (the storage trait, the check,
     the modes, sign-in per server, the tests and the order of the pull requests).
   - The work lands on `main`, one pull request at a time; the new server kinds stay hidden and refused unless
     `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` is set, until the 0.3.0 release turns them on.
9. **No Python in the tree (M10), part 1, for 0.3.0:** `scripts/create-owner`, `add-user`, `app-password` and
   `smoke-test` read and write JSON through `python3 -c` one-liners; they read and write it with sed instead, so they
   run without Python (sh and curl). `packaging/icons.sh` (Pillow) becomes
   `cargo xtask icons`.
10. **No Python in the tree (M10), part 2:** the design build, about 7,000 lines in `design/tools/*.py` that drive
   `figcraft-cli` over MCP to make `design/craftcloud.figcraft` and its exports, becomes `cargo xtask design`,
   building on craft-libs' `jsonrpc` and `mcp` crates. Until then it is the one Python left (AGENTS.md).

## Progress log

- 2026-11-26: Desktop: the client says when a new version is out. Once a day (from the first frame the window
  draws, never for a command, and not while it waits hidden in the tray) it reads the project's GitHub release
  list through craft-libs' `craft-update` (default features: check only, nothing downloaded or installed) and
  Settings › About says "CraftCloud <version> is out." with a link to the release page, or, for a deb, rpm, Arch,
  Flatpak, Homebrew or Nix copy, to update it with the package manager. Settings › On this computer has
  "Check for updates" (on unless switched off; older settings files read as on); the last look and its find
  are kept in `update-check.json` beside the settings. Failures and GitHub's request limit are only logged.
  Tested against recorded release lists (newer, same, older, a pre-release, a draft, the request limit) with no
  network. craft-update's gaps are drafted in [UPSTREAM.md](UPSTREAM.md) (F3).
- 2026-11-26: Build: `cargo xtask ci` runs its steps on craft-libs' `craft-xtask` runner, the Craft apps' gate
  runner, instead of its own loop. The steps, their commands and the summary are as before; a passing summary
  still names the tree it checked (`… in <path>`), which CraftCloud adds to craft-xtask's line.
- 2026-11-26: CLI: what a cloud backend that runs `craftcloud … --json` needs. Files listed from the server (`ls`,
  `files` without a client, `starred`, `recent`, `search`) carry their version tag `etag` in its HTTP form, from the
  listing already made (OxiCloud computes the REST and WebDAV tags with one formula). `save --if-match ETAG` writes
  only over that version: straight to the server, the old version kept first, `If-Match` on the write; a file that
  changed, moved or went meanwhile is exit 5 (`conflict`) with nothing written or kept. `save` and `put` answer with
  the new `etag`. `save` without `--force` already creates only (`If-None-Match: *`), and `get PATH -` already
  streams the bytes alone to stdout; both are now documented as the contract (CRAFT-INTEGRATION.md § 3.5). Tests:
  the write decision (`files::plan_write`), the tag forms and JSON shape, the flags, and the whole round trip in
  `cli_live.rs` (CI's `live` job).
- 2026-11-26: Sync: a finished download is put in place through craft-libs' `craft-atomic`. A rename another
  program holds up for a moment (on Windows, a virus scanner reading the new part file) is tried again, with the
  last look at the file here before every try, so a file saved here meanwhile is never overwritten; before, the
  download was thrown away and fetched again. The folder is flushed after a file is put in place, and
  `state.json`, `storage-check.json` and the token file are written crash-safe (flushed before they replace the
  old one). Tests: a held part file goes in place once let go; a file changed while the rename waits is kept.
- 2026-11-26: Docs: how the Craft apps would reach CraftCloud through craft-libs. CRAFT-INTEGRATION.md § 5 now
  builds on `craft-cloud-ui`'s `CloudBackend` (the `craft-cloud` name is taken by the vendor-neutral contract): a
  backend in craft-libs that runs `craftcloud --json`, with each job mapped to a command, what stays unsupported
  and the gaps (no rename-stable id over WebDAV, no permission on a link). The command-line half it needs (an ETag
  in `files --json`, `save --if-match`, a read to stdout) is its own change. § 5.1 says why CraftCloud's sign-in
  stays out of `craft-suite-account` for now. UPSTREAM.md gets a craft-libs section (F1, F2), drafts, not filed.
- 2026-11-26: Deps: craft-libs (`craft-dav`, `craft-xml`, `craft-settings`) on `de994d8` and the egui fork on
  `b977090`, the revisions craft-launcher builds with (from `8ebdc49` and `6d518fc`). `craft-settings` now brings
  `craft-host` and `craft-i18n` (and with them the ICU 2.3 crates and `chrono`) into the desktop build; the web
  build is unchanged. `chrono`'s "unmaintained" advisory (RUSTSEC-2026-0543) is ignored in `deny.toml` with its
  reason, like the D-Bus ones. First of the craft-libs moves (craft-xtask, craft-atomic, craft-update, the
  craft-cloud backend, a craft-ui spike).
- 2026-11-23: Craft apps: LoveCraft's web build (0.2.0, native bridge) in the `-apps` image. Its release names it
  `lovecraft-web-<version>.zip` while its id stays `prosecraft-rs`, so `apps.toml` takes a `release_name`, which
  `bump-apps` looks for and the manifest check holds `asset` to; the pins were set by `bump-apps`.
- 2026-11-23: Craft apps: LoveCraft (`prosecraft-rs`, `.prosecraft`) in `apps.toml` and the built-in catalog, as
  the first desktop-only app: `web = false` means no pins and no bridge, `fetch-apps` leaves it out of the `-apps`
  image and `bump-apps` passes it by. LoveCraft uses `files --app prosecraft-rs` and `save` for its Open from
  and Save to CraftCloud.
- 2026-11-22: Web (M5, for 0.3.0): an **Apps** menu in the web header. When the server has the Craft web apps on
  and at least one switched on (`/craftcloud/config.json`'s `web_apps` and `enabled_apps`, now read by
  `core::RuntimeConfig`), a signed-in person gets APPS between FILES and ADMIN (the app-grid glyph alone on a
  phone); it drops a grid of the apps' icons and names, and a tile opens `/apps/<id>/` in a new tab. With the
  apps off or none on, the header is unchanged. Saving the web apps in setup or Admin › Server reloads the config,
  so the menu follows without a page reload. Keyboard: the focus goes to the first tile, the arrows, Home and End
  move, Enter opens, Esc closes and returns the focus to APPS; APPS reports its expanded state and the tiles are
  links. Design frames `v2-web-apps-menu` (and `-light`, `-phone`, `design/tools/v2_apps.py`); tests of the
  visibility rule and the keyboard paths in `crates/ui-egui/src/web/apps_menu/tests.rs`, and the web shots
  `apps-closed`, `apps-menu`, `apps-off`.
- 2026-11-22: Sync and desktop: a folder move that keeps failing is never given up, and says so. The engine counts
  the failed tries of each waiting folder move (`SyncProgress::waiting_moves`): an error line in the activity list
  after the first, a warning line after the third (`MOVE_WARNS_AFTER`), a last one after the fifth
  (`MOVE_LAST_NOTICE_AFTER`), nothing after. Tries one to three come one a pass, the fourth after 2 minutes, every
  later one after 5 (`core::sync::move_retry_wait`; "Sync now" and `craftcloud sync` ask at once); a success, or a
  move that is no longer planned, clears the count. From the third try the window says "A folder move is waiting"
  with a panel and "Try again", the tray shows its waiting look, and a notification goes out; a last notification
  after the fifth. `status --json` and `sync --once --json` list `waiting_moves` with `attempts`. Tests: the
  thresholds, the waits (a test clock, no sleeping), the reset, over both drivers (a second golden log); the
  notifications, the tray's look, the window and the status output.
- 2026-11-19: Sync: a move the server can't do right now is tried again, never rebuilt as a copy. Found by the live
  suite on Nextcloud, which answered 500 ("locked") to the `MOVE` of a renamed folder while another device was
  writing into it; the client then built the folder again beside the old one. `storage::refused_for_good` names
  the answers that refuse a move for what it is (400, 403, 404, 405, 409, 412, 414, 422, 501: files then sync one
  by one, as before); every other answer, and no answer, is "not now": the folder is set aside for the pass under
  both names (nothing uploaded, downloaded, trashed or forgotten, no mass delete asked about, the state kept), said
  once in the activity list, and moved by one `MOVE` on a later pass. One renamed file likewise. On CraftCloud's
  own server too. Tests: the scripted servers answer a `MOVE` or a `PUT` with a status once
  (`Flags::answers_once`), over both drivers, and a new golden log; the two live tests that failed sync until
  nothing changes (`settle`) instead of counting passes, since two computers are at the server at once there.
- 2026-11-18: Sync (M9, pull request 6 of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14): Nextcloud. A profile hook
  on the generic driver (`storage::profile::ServerProfile`: the server's version string, the probe's known-server
  checks of steps 16 and 17, "verify writes by checksum", the server's own trash, versions and quota endpoint) and
  the first profile, Nextcloud's (`profile/nextcloud.rs`, with `core::api::nextcloud` for the requests and parsers,
  recorded from Nextcloud 31 in `crates/core/tests/fixtures/nextcloud-31/`). `craftcloud login <server> --kind
  nextcloud` signs in with Login Flow v2 (the browser, an app password made for this device) or an app password
  with `--user`; the user id comes from `current-user-principal`; `logout` deletes the app password on the server;
  without `--kind`, `status.php` suggests the kind. The check on connect's step 16 deletes the probe's file, finds
  it in the trash bin and restores it, step 17 overwrites a file and finds an earlier version by id; only then are
  the server's trash (a `DELETE` into it, a file's with `If-Match`) and versions (`keep_version` and
  `move_versions` no-ops) used; the runner allows on the trash bin's root only that restore, never a `DELETE`. The
  maintainer's decision Q13: Nextcloud reuses a file's version tag within one second, so every `PUT` carries
  `OC-Checksum: SHA1` and is read back, and a write landing under our tag is a conflict copy (tested without the
  check: it lands silently; with it: both kept). A version restore is a guarded write (the version's bytes read,
  then a `PUT` with `If-Match`; a change meanwhile restores nothing, code 5), never Nextcloud's own restore
  `MOVE`. `trash rm|empty` on the server's trash answer code 10 (Q8).
  UPSTREAM.md E1 proposes a fresh ETag on every write. All behind the opt-in; OxiCloud's golden logs unchanged.
- 2026-11-18: Tests (M9, pull request 5 of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14): the live suite against
  other servers. `crates/native/tests/live.rs` takes `CRAFTCLOUD_TEST_KIND` (`oxicloud` by default, the suite CI
  runs, unchanged; `nextcloud`, `owncloud`, `opencloud`, `webdav`, each only with `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`,
  a run naming a kind it can't run failing at once): the same tests go through the storage trait, with the cloud
  side per kind (OxiCloud over REST as before; the others through the generic driver, an app password made the
  way a person would, Nextcloud's OCS call or ownCloud's Settings › Security, never the account password for the
  sync), per-kind deadlines (Nextcloud's about 4 uploads a second), and the sync mode asserted on every pass: a
  server that got better or worse fails. The two-way tests run where two-way sync is expected, the download-only
  suite of § 9 where download only is (nothing written to the server, a local change kept and said once, a
  cloud change over a local change a local conflict copy, a cloud delete into the local `.craftcloud/trash/`,
  also after a mass-delete hold, a failed listing holding everything under it through a proxy that fails one
  folder's `PROPFIND`). `apps/craftcloud/tests/cli_live.rs` runs `login --kind webdav`, `server check`, the file,
  trash and version commands and the `unsupported` answers (exit code 10) on such a server.
  `tests/other-servers/compose.yaml` (images pinned by digest, loopback only, passwords from the environment)
  starts Nextcloud 31, ownCloud 10.15, OpenCloud 9.1.1, `rclone serve webdav` 1.76.0 (download only: it ignores
  the `PUT` guards, Q2) and Apache httpd 2.4.71 with `mod_dav` (two-way with the client's own trash and versions;
  measured with the check on connect: every guard honoured once its ETags are content digests, since its default
  time-based tags are weak for a second after a write and failed step 6). § 5's figures are reproducible from the
  repository now, with the two plain servers as new columns; OpenCloud 9.1.1 turns out to make app tokens for a
  person (`auth-app`, on by default), which the suite uses. Found on Apache by the suite's first run and fixed in
  pull request 4b: the generic driver named a folder without its trailing slash when it restored or purged a trash
  item, Apache redirected that (301), the redirect was followed as an unauthenticated `GET`, and the 401 read as a
  revoked password. No CI job (§ 14 row 9).
- 2026-11-18: Sync (M9, pull request 4b of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14, the second of two under its
  row 4): the client trash's retention, the runtime watch, and the trash and version commands on any WebDAV server,
  all behind the opt-in. The generic driver's `sweep_trash` removes client-trash entries older than 30 days by the
  **server's** clock (each entry's `getlastmodified` against the listing's `Date`, never this computer's clock; a
  shared pure `core::storage::probe::older_than` with the 30-day `dav::TRASH_RETAIN_SECS`), through `purge_own` only,
  at most once a day per client, after a cycle; the engine calls it, a server with its own trash is a no-op. The
  command line routes `trash ls|restore|rm|empty` and `versions`/`versions … restore` through the trait on such a
  server (reading needs the mode's ok; writing needs two-way, `webdav_for_writes`): `trash ls` shows the original
  path and when it was trashed, `restore` is a `MOVE` back with `Overwrite: F` (`names::restored_name` on a clash),
  `empty` and `rm` purge inside `.craftcloud/trash/` only, versions share OxiCloud's `.craftcloud/versions/` layout
  and code. The runtime watch (engine): a file this client just wrote that comes back as someone else's (the ETag
  the write answered isn't the one listed and the size isn't ours) stops two-way sync for the rest of the run (the
  mode drops to download only), re-runs the check (`RemoteStorage::recheck`), saves it, and says so once in the
  activity list and in `status`. `WebDav::connect` now reuses the saved check only while it is less than a day old
  and the server's version string (a plain WebDAV `Server` header) hasn't changed, else re-probes and saves. Tests:
  retention by the server's clock (a day-old entry stays, a 31-day one goes, no `Date` sweeps nothing, nothing
  outside `.craftcloud/trash/` touched), `trash empty` purges only inside it, versions listed and restored on the
  generic driver, the watch's two triggers (mode drops, probe re-run, activity line, nothing more written after),
  the re-run schedule (fresh: no probe; version changed: probe; day-old: probe), and `older_than` over any span. The
  15 golden logs are unchanged.
- 2026-11-18: CLI: `craftcloud versions PATH restore N` never overwrites a newer file. The restore was a `COPY` of
  the kept version over the file with `Overwrite: T`, which has no guard: a write that landed between the snapshot
  and the `COPY` (another device, the web, a Craft app saving) was lost, with no version of it kept. It now does
  what the web's restore does: the file's ETag first, the version's bytes, the current file kept as a version, then
  a `PUT` with `If-Match`; a 412 keeps the change and says so (exit code 5). The unguarded `COPY` builders
  (`versions::restore`, `dav::copy_over`) are gone. The sync engine never called it: its decisions are unchanged.
  Tests against the scripted server, one of them with a write landing right after the snapshot.
- 2026-11-18: Sync (M9, pull request 4a of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14, the first of two under its
  row 4): any WebDAV server, with the client's own trash and versions, behind the opt-in. The opt-in
  (`CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`, read in `native::experimental` and asked by the command line, the daemon
  and the desktop client): without it `--kind` is hidden and `--kind webdav` a usage error, and a settings file
  naming another kind of server is refused with one message, nothing written (tests on the built program with the
  variable unset, `0`, `true`, `yes` and `1`). `Credential` (`Bearer` or `AppPassword`; OxiCloud's keychain entries
  keep 0.2.0's exact bytes) and `AppPasswordSession` (Basic auth, a 401 is signed out, never a password over plain
  http to another computer). The settings gain `storage` and `dav_url` (a 0.2.0 file reads as `oxicloud`). The
  generic driver (`storage::webdav`): listings read against its base, guarded creates and overwrites, `still_at`
  and `cloud_holds` before a move or trash, the trash by `MOVE` into `.craftcloud/trash/<time>-<device>/<path>`
  (a folder as one `MOVE`), versions by `COPY` through the code OxiCloud's driver now shares (`storage::versions`),
  free space from the quota props, `DELETE` only through `purge_own`; `WebDav::connect` reads the saved check or
  runs the probe. The engine builds it on its sync thread (`sync::start_webdav`), says the mode (`SyncProgress.mode`,
  `status --json`'s `mode`, "Download only" in the desktop's words), says once what download only doesn't upload
  and lists it as `not_uploaded`, and holds local names a case-insensitive server can't keep apart. The command
  line: `login <WebDAV address> --kind webdav --user NAME` (the password from `CRAFTCLOUD_APP_PASSWORD` or a
  prompt), `server check` runs the probe on such a server, `ls`/`get`/`put`/`save`/`mkdir`/`mv`/`rm`/`files`/`sync`
  through the trait, exit code 10 (`unsupported`) for what CraftCloud's server alone has. A sync folder holding
  another sync client's database is refused (code 5). The desktop client says to use the command line for such a
  server until pull request 10. Tests: the driver and the engine over it against the scripted WebDAV server (every
  write's guard and the password, hrefs outside the base, the trash and restore round trip, a folder whole,
  versions kept, pruned and moved, free space, the refused password, connect probing once, a two-way cycle, a
  download-only cycle that writes nothing and says so once, the trash guards, a failed listing), the credential's
  bytes, the opt-in on the binary, the clash rule, the folder refusal, exit code 10. The 15 golden logs are
  unchanged. Left to 4b: the trash's retention and `trash empty`, `trash`/`versions` through the trait, the probe's
  re-run schedule, the runtime watch.
- 2026-11-18: Sync (M9, pull request 3 of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14): the server check. The check
  on connect is a state machine in `core::storage::probe` (§ 7.5's twenty steps as request/expectation pairs, every
  write with its guard, the verdict as `Capabilities` plus each step in plain words, `Mode` through `mode_for`; a
  probe that doesn't finish, a version tag that doesn't change with the bytes or a listing without one is refused),
  with the sweep of old probe folders decided by the server's clock alone and the saved check's shape
  (`storage-check.json`, every field with a safe default). `native::storage::probe` runs it over a small transport
  trait (`Session` implements it; the generic driver will too), checks every request's guard again before sending,
  and refuses a probe that takes too long. `craftcloud server check [--json]` prints the capability table and the
  mode; on CraftCloud's own server no probe runs (Q3): the known capabilities, marked not measured. Tests: the
  verdict for each misbehaviour of § 5 against a scripted WebDAV server (`core::storage::probe::fake`, also behind
  the loopback `fake_server` in `native`), the sweep rule, the request-log guards, a server that stops answering
  (refused, swept next time), the saved check's round trip, a property test that one more failing step never raises
  the mode; `cli_live` checks `server check` and that nothing was written. Steps 16 and 17 (a known server's trash
  and versions) take a profile's check through `ExtraCheck`; none exists yet, so the verdict records CraftCloud's
  own trash and versions. The hard names of step 18 are recorded in words; the name rules stay the strict ones.
  Found on the way: Nextcloud and ownCloud keep a file's version tag when it is overwritten within the same
  second as the write before it, so the probe waits a second before its two overwrites (noted in the design's
  as-built note for the maintainer). Nothing changes for an OxiCloud sync: the 15 golden logs are unchanged.
- 2026-11-17: Sync (M9, pull request 2 of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14): sync modes. What a server
  guarantees decides what the sync may do with it (`core::storage::mode_for`: two-way, download only, or refused),
  and every plan is cut to that mode (`core::reconcile::restrict`): download only never uploads, moves, makes,
  removes or trashes anything in the cloud, keeps a change made here where it is, sets a local conflict copy aside
  without uploading it, and moves a file deleted in the cloud (once past the mass-delete hold) into
  `.craftcloud/trash/` in the local folder instead of deleting it. The names a server can't take are data
  (`names::NameRules`, OxiCloud's exactly as before) and go through the one skip rule, `names::skipped`. Nothing
  changes for a person: OxiCloud is always two-way, which cuts nothing (the 15 golden logs are unchanged), and no
  other server kind reaches the engine yet. Property tests check the cut against generated plans and carry
  download-only plans out cycle by cycle (the cloud never changes, no change made here is lost); engine tests run a
  whole download-only cycle against the fake server (only listings reach it).
- 2026-11-17: Agents: the guarded-writes rule says where old bytes are kept per server (the server's own versions
  where the check on connect confirms them, else `.craftcloud/versions/`) and that a server ignoring a guard never
  syncs both ways, as the WebDAV design decided.
- 2026-11-17: Sync (M9, pull request 1 of [WEBDAV-STORAGE.md](WEBDAV-STORAGE.md) § 14): the engine talks to the
  server only through a storage trait (`RemoteStorage`, `crates/native/src/storage.rs`) with no unguarded write, and
  OxiCloud is its first driver (`storage/oxicloud.rs`, today's requests moved behind it); what a server guarantees
  is `core::storage::Capabilities`, OxiCloud's filled in by hand. Nothing changes for a person: 15 scenarios'
  requests, activity lines and saved state were recorded against the old engine (`crates/native/src/sync/golden/`)
  and the new one sends exactly the same; every recorded write carries its guard.

- 2026-11-17: Server scripts: `scripts/create-owner`, `add-user`, `app-password` and `smoke-test` need only sh and
  curl, no Python. A JSON string is escaped with sed (`\`, `"`, control characters as `\u00XX`; other characters
  pass through as they are), and the answers' few fields are read with sed. Passwords now reach curl on its stdin,
  not in a command line. `the_operator_scripts` in `cli_live.rs` (CI's `live` job) drives the three against the
  server with a failing `python3` first in `PATH` and a password with `"`, `\`, `$` and letters outside ASCII; it
  fails with the old scripts.
- 2026-11-17: Roadmap: M10, no Python in the tree: the operator scripts and the packaging icons in Rust for
  0.3.0, the design build later; AGENTS.md names `design/tools` as the one Python until then.
- 2026-11-17: Build: the packaging icons (the Linux hicolor PNGs, the Windows .ico, the macOS .icns) are rendered
  from the logo by `cargo xtask icons`, in Rust; `packaging/icons.sh` and its Python/Pillow need are gone. The
  committed icons were rendered again with it: the same files, sizes and entries, Lanczos on premultiplied alpha
  as before, within a few levels of the old ones (mostly smaller files).
- 2026-11-17: Docs: the design for M9, any WebDAV server as the storage ([WEBDAV-STORAGE.md](WEBDAV-STORAGE.md)),
  from the servers' answers measured on 2026-11-16 and a throwaway spike against Nextcloud, ownCloud and
  OpenCloud. The maintainer took its recommendations after a research pass; the work lands on `main` behind
  `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` until the 0.3.0 release. AGENTS.md's sign-in rule now reads "never the
  account password", and its delete rule "a delete never removes a file for good", with how each server meets it.
- 2026-11-17: Web: the version a page shows before the server's config arrives is this build's own, not a fixed
  "0.1.0"; the README's screenshots (`docs/screenshots/`) are rendered again by `web_shots`, which now takes the
  version from the build too, so they say 0.2.0.
- 2026-11-16: Version 0.2.0: share links, the public share page and version history on the web, single sign-on
  setup in plain words, Linux notifications, the Windows fixes found testing 0.1.1, the server hardening, a Home
  Manager module, the sync fixes the property tests and the engine tests found (upgrade all devices together), and
  the sign-out and trace-log fixes. Tests: property, engine, mixed-version e2e, e2e-web, live tests in CI and on
  Windows. M9 (any WebDAV server) moves to 0.3.0.
- 2026-11-17: Desktop (Linux, X11): a start at login (`--hidden`) stays in the tray, and opens the window where
  there is no tray. X11 had the Windows problem below (eframe shows the window after its first frame), and the
  same rule hides it again; once unmapped, X11 sends no new `VisibilityNotify`, so eframe still takes the window
  as drawn and the rule (and `ui` drawing nothing) holds in every pass. On top, a window that starts hidden counts
  as shown when there is no tray to come back from (no tray host, or no icon on Windows and macOS), so it opens
  instead of leaving a client nobody can reach, and every hidden start gives the Linux tray up to 3 s before the
  window is made, so that is known. `a_hidden_start_stays_hidden_where_there_is_a_tray` fails without it; the
  Windows test now runs with a tray up. Found in a headless Xfce session while checking the desktops.
- 2026-11-16: Desktop, Windows: a start at login stays in the tray (#9), and a window hidden in the tray draws
  nothing. eframe creates every window hidden and calls `set_visible(true)` itself after the first frame it paints
  (`post_rendering`), whatever `with_visible(false)` asked; and winit's Windows backend never reports occlusion, so
  `ViewportInfo::visible()` is `None` for a hidden, unminimised window and eframe runs `ui` and paints it at every
  tick (at least every 100 ms while a repaint is pending). Now `logic` sends `Visible(false)` in every pass where
  the window isn't shown but eframe takes it as drawn (the pass's commands run after `post_rendering`, so the
  window is hidden again in the same step), and `ui` draws nothing while the window isn't shown; Show still
  brings it back in the same pass. `a_hidden_start_keeps_the_window_hidden_and_draws_nothing` fails without the
  fix. `craftcloud status` now tells a name Windows can't have, and a case clash, from a name the cloud can't
  take. Not yet checked on the Windows VM.

- 2026-11-16: Desktop (Linux): the tray counts as up only while a tray host owns `org.kde.StatusNotifierWatcher`
  on the session bus. The StatusNotifierItem backend registers the icon whether or not anything shows it (a
  bar may start later), and the client took that registration as "the tray is up": on stock GNOME (no
  AppIndicator extension) closing the window left it running with no icon and no window, and a start at login
  showed nothing. The host is now followed on a thread of its own (`NameHasOwner`, then `NameOwnerChanged`):
  with none, closing the window quits and `--hidden` opens the window; a host that comes up later makes the
  icon count. A unit test runs a private `dbus-daemon` and claims the name. Found in a headless GNOME Shell
  container while checking the desktops (`docs/LINUX-DESKTOPS.md`, its own pull request).
- 2026-11-16: Docs: the desktop client on KDE Plasma 6.8, GNOME 51 and Xfce 4.20, in headless containers
  ([LINUX-DESKTOPS.md](LINUX-DESKTOPS.md)): the tray's registration, notifications from each desktop's own
  service, the keychain (GNOME Keyring, KWallet's Secret Service, none), the `craftcloud://` handler and
  `xdg-open`, the autostart entry, and the window drawn in software. Found: on X11 a start at login opens the
  window, and with no tray host (stock GNOME) the client takes the tray as up, so closing the window leaves it
  invisible; both have fixes of their own. A real login, the lock screen and the KDE panel stay untested.
- 2026-11-17: Desktop: Settings' "Pause syncing" switch (Off | On, where Off meant syncing) is now "Syncing" with
  Paused | Active. The design (`design/`, rebuilt with FigCraft 0.18.1) matches it and shows version 0.2.0.
- 2026-11-16: CI: the live sync tests run on every pull request and push to `main`. A `live` job in `ci.yml`,
  beside the gate, builds the server image from the tree, starts it with `scripts/smoke-test` and runs
  `cli_live.rs` and `live.rs` against it (the server's logs kept as an artifact when they fail), so a change to the
  engine is checked against a real OxiCloud before it is merged, not only before a release.
  Its first runs: `live.rs` 20 of 20 in 3 minutes, the job about 9 minutes. `cli_live` failed both times at its
  versions check: a file replaced within a second of being written lists a version `modified` a second after its
  `replaced_at` (OxiCloud's Last-Modified is a whole second, the client's clock isn't), for its own fix.
- 2026-11-16: Desktop: an idle window uses next to no CPU (#3). On the main screen `DesktopApp::tick` asked for a
  repaint every 250 ms, and `logic` for one every second, so an open window drew 4 full frames a second for ever;
  a software renderer draws them on the CPU. Now the 250 ms tick runs only while a request is in flight, an idle
  main screen asks for one a minute (`IDLE_TICK`, for "Synced 2 min ago"), and `logic`'s 1 s tick runs only while
  the window is hidden or minimised (where eframe calls only `logic`, so the tray's Show keeps working); the sync
  engine, the network, the tray and the control channel wake the window themselves. Measured on Linux with Mesa's
  llvmpipe (software GL, X11, signed in, idle on Activity, 60 s): 4.1 passes a second and 22 % of a core before, 0.2
  (three per 15 s sync check) and 1 % after. `an_idle_window_repaints_rarely_and_a_busy_one_often` fails without
  the fix; the Windows re-check is still to do.
- 2026-11-16: Command line: a server that can't be reached gives exit code 8 on every system. The HTTP layer
  (`craftcloud_native::http`) told an unreachable server from other errors by the system's English message
  ("Connection refused"), so on Windows ("No connection could be made …", os error 10061) `logout` without a server
  exited with 1; found by the new Windows tests job in the Release workflow (`cli_logout`). It now reads the
  error's kind (`ErrorKind::ConnectionRefused`, `ConnectionReset`, `TimedOut`, `HostUnreachable` …, ureq's
  `HostNotFound`, `ConnectionFailed` and connect timeouts, Winsock's lookup codes), and the CLI asks
  `http::is_unreachable`. A name that can't be looked up was missed on Linux too. The new `http` tests with
  Windows- and Linux-shaped errors and a real lookup of a `.invalid` name fail without the fix.
- 2026-11-16: Tests: the sync's skip-rule property test (`the_skip_rule_holds_whole_folders`) passes on Windows.
  It checked that a cloud path becomes a plain local path under both name rules, and on a Windows computer Linux's
  rule let `a:` through, which Windows reads as a drive. The client itself always uses the rule of the computer it
  runs on (`local_relative` and the sync engine take `Platform::current()`; the one other use, the "won't sync to
  a Windows computer" warning, builds no path), so the test now checks the local path with that rule only.
  Found by the new Windows tests job in the Release workflow.
- 2026-11-16: Releases: the Craft web app pins are checked before a release. [RELEASING.md](RELEASING.md)'s checklist
  starts with `cargo xtask bump-apps --check` (a stale pin is moved in a pull request of its own, built and
  smoke-tested as the `-apps` image), and the Release workflow runs the same check in its Linux x86_64 job and writes
  the result to the run's summary ("Craft web app pins"), without ever failing the release on it.
- 2026-11-16: Desktop: an offline main window no longer asks `/api/auth/me` on every frame. A failed answer (no
  connection, a 5xx, a body that names no user) left `me` unknown, the answer woke the window, and the next `tick`
  sent the request again at once, so an offline window requested and redrew without pause. Now a failure sets a
  wait (`me_retry`, modelled on the device poll's `next_poll`): 5 s, doubled on every failure up to 2 min, reset on
  an answer that names the user and on a sign-in change; during the wait the window asks for one repaint at the
  retry time. `who_is_signed_in_is_asked_again_after_a_wait_that_grows` fails without the fix.
- 2026-11-16: Versions: a kept version's content time is never shown after its replacement time
  (`versions::Version::modified_at`, which the command line and the web frontend both use). OxiCloud rounds
  `getlastmodified` to the nearest second (UPSTREAM.md C17) and the replacement time is the client's clock, so a
  `put --force` within half a second of the first `put` showed `modified` a second after `replaced_at`; a server
  clock ahead of the client's did the same by more. Found by `cli_live` in the new CI `live` job.
  `a_versions_content_is_never_newer_than_its_replacement` fails without the fix.
- 2026-11-16: Logging: trace logs no longer include HTTP request bodies and headers. The desktop client, the
  command line and `craftcloud-admin` log through one filter (`craftcloud_core::logging::WireCapped`) that stops the
  HTTP library (`ureq`, `ureq_proto`) at debug level whatever `RUST_LOG` asks for. `tokens_never_reach_the_trace_log`
  runs again, and `trace_logging_leaves_out_the_tokens` runs `craftcloud logout` with `RUST_LOG=trace` against a
  stand-in server.
- 2026-11-16: Sync: a cloud file listed without an ETag is never trashed. The check before a trash (`still_at`)
  took a file listed without an ETag as unchanged, and the folder check (`cloud_holds`) did too when the plan's
  ETag was empty as well; both now take it as changed, so it stays in the cloud and comes back down, said once in
  the activity list (a move here is then an upload under the new name). Tests in
  `crates/native/src/sync/engine_tests.rs`: `a_file_listed_without_an_etag_is_not_trashed` (no longer ignored) and
  `a_folder_with_a_file_listed_without_an_etag_is_not_trashed`. Upgrade all devices together.
- 2026-11-16: Tests: `cargo xtask e2e-web` runs the web frontend in headless Chromium (chromedriver, through a
  small WebDriver client of its own over `ureq`; no new dependency) against a fresh server: first-run setup, sign-out and sign-in with the session cookie's flags,
  an upload, a share link opened in a second, signed-out browser and downloaded there, a device code approved on
  `/device`, a new user on Admin › Users with the CSRF header, and forged admin writes refused. It works the canvas
  with the keyboard and checks the server, the cookie jar and the network log ([TESTING.md](TESTING.md)). Not in CI.
- 2026-11-16: Tests: the top three gaps of [TESTING.md › Coverage](TESTING.md#the-gaps-worth-a-test-by-risk), with
  the real engine and session against a scripted local server (`crates/native/src/fake_server.rs`), in
  `cargo xtask ci`: switching a folder off keeps files changed since the last sync (size or time) and says so; a
  file or folder changed in the cloud since the plan, or whose listing fails, is never trashed; an expired token
  is refreshed once and the request sent again, another process's tokens are taken over (and refreshed with their
  own refresh token when stale), a refused refresh (401, 403, 404) signs out. `crates/native` 55% → 67% of lines,
  `session.rs` 13% → 73%. Two known bugs, each an ignored test, not fixed here: a cloud file listed without an
  ETag is trashed (`still_at`), and at trace level ureq-proto logs a refresh's tokens.
- 2026-11-16: Sync: one file is in one move at most. A file moved here onto a synced file that was renamed in the
  cloud was taken both as that move and as the rename (two moves through one path); the move here now wins and the
  renamed cloud file comes down as new. The other way round (a file moved onto another in the cloud while that one
  was renamed here), the cloud file is no longer moved to the new name, where the next pass downloaded it over the
  renamed file. `pair_moves` in `crates/core/src/reconcile.rs`; plan tests in `reconcile/tests.rs`, end-state
  tests and the properties (no more exclusion, 50 000 cases) in `reconcile/properties.rs`. Upgrade all devices
  together.
- 2026-11-16: Sync: a folder replaced by a file (or a file by a folder) syncs in one pass. The plan kept its
  usual order (folders to make, files, removals) and only the new file at the folder's place waits until the
  folder is gone, so a file moved out of the folder goes before the folder is removed and after the folder it
  moves into is made; a file replaced by a folder is deleted first. A file moved to where its own folder was (or
  into a folder in its own place) is a delete plus a new file, since no single move can do it (it used to fail on
  every pass), and a folder a set-aside path was leaving is no longer "removed" while still full. A new property
  test carries each generated plan out once and checks that every action's preconditions hold in plan order
  (`a_plan_works_in_one_pass`); the ignored test `a_replaced_folder_is_emptied_before_it_goes` runs again.
- 2026-11-16: Desktop: the tray checked item by item against the v2 tray frame (#25). Order, labels, separators,
  the status line, Recent files, the folder-missing items and the four icon looks matched; the pause item didn't:
  signed out and waiting for you (mass delete) share the paused icon, so the menu said "Resume syncing" and the pick
  paused. "Resume syncing" now shows only while paused, as in the frame (`tray.rs`, `MenuState`, tested). Not in
  the frame yet: the tray dot stays green for "1 conflict" and "N not synced", where the header's dot is amber.
- 2026-11-16: Sync: property tests of the decisions (`crates/core/src/reconcile/properties.rs`, proptest). Generated
  states, scans and listings check the rules in AGENTS.md: only unchanged synced files are deleted, never on a side
  that changed; writes name the version they replace; changes in two places become a conflict; a folder goes to
  the trash whole only as it was synced; mass deletes wait; a place held as last synced (unreadable, or a missing
  sync folder) is never deleted, moved away or forgotten in the cloud; a skipped name is never planned; and plans
  carried out cycle by cycle settle with both sides the same and no change lost. They found two rare plans, not
  fixed yet (each with an ignored test in `reconcile/tests.rs`): one file taken as two moves, and a folder replaced
  by a file removed before files are moved out of it.
- 2026-11-16: Tests: a coverage run (`cargo llvm-cov`, by hand, not in CI) before 0.2.0: 71.5% of lines without the
  live tests; `crates/core` 96%, `crates/native` 55%. [TESTING.md › Coverage](TESTING.md#coverage) has the commands
  and the ten gaps worth a test, by risk (first: switching a folder off with a changed file in it).
- 2026-11-16: Testing: `cargo xtask e2e-sync` runs a mixed pair, each client its own program (`--bin-laptop`,
  `--bin-studio`), for the "upgrade all devices together" risk. New cases for every run: a conflict with each side
  syncing first, a folder deleted from the Studio, a version kept by one client and restored by the other (both
  ways), names Windows can't have synced both ways and never taken as deleted, and leftovers (part files,
  `.craft_tmp`, `.craftcloud/`) that never reach the other side or the cloud. With an admin account the cleanup
  checks that the clients' `logout` ended their sign-ins on the server, and the run's own password sign-in now
  ends with its refresh token. Run on Linux against a throwaway
  server with 0.1.1 on one side and this tree on the other, both ways round: every sync case passed. The Windows-only
  name rule of 0.2.0 isn't exercised on Linux ([TESTING.md](TESTING.md#mixed-versions-an-older-release-on-one-side)).
- 2026-11-16: Tests: the live sync tests (`crates/native/tests/live.rs`) no longer depend on what is on the test
  account. Each test syncs only top-level folders of its own (`live-<test>-<seconds>-<pid>-…`), with every folder
  the account had before switched off, and when it ends, failed or not, its folders and kept versions go to the
  trash and from there for good. A whole run, or one test twice in a row, passes on an account with leftovers (an
  old `t5/`, a conflict copy in `Documents/`). The offline computer in the rename-and-edit test is paused while
  offline, so its folder watcher no longer races the other computer.
- 2026-11-16: Testing: the Release workflow runs the workspace tests on Windows x64 (`windows-test`), so the
  Windows code is tested on Windows itself and not only on Linux; on `v*` tags and by hand, not on every pull request.
  No draft release unless they pass. Its first run found one test that built a path with `/`; the app was right.
- 2026-11-16: Sign-out: `craftcloud logout` and the desktop client's Sign out end the session on the server too
  (the logout now carries the device's refresh token; before, the server answered 401 and the session stayed).
  When the server can't be told, the sign-in is still forgotten locally and the CLI (`server_session_ended`, exit
  8 or 1) and the desktop client (a note) say so. Found by the mixed-version e2e test.
- 2026-11-16: Roadmap: M9, sync with any WebDAV server (Nextcloud, ownCloud, a NAS …), planned for 0.2.0 on the
  branch `feature/webdav-support`. The Windows command line (`craftcloud.com`, the PATH entry, the uninstall's
  cleanup) leaves Upcoming: it was tried on Windows 11.
- 2026-11-15: CLI: on Windows, `craftcloud open` on a file whose type has no app (`.md`, `.docx` on a fresh
  Windows) shows the "Open with" dialog instead of nothing, and `--json` says `"with": "chooser"`.
- 2026-11-15: Web: the file browser's **details panel** (#24). On a wide page, the one file selected shows beside the
  list: a preview (the image, or the mark of the Craft app that opens it), its name and star, "DOCX document · 248 KB
  · opens in WordCraft", DOWNLOAD, SHARE… and the "…" menu, when it changed, where it is, its owner and whether it is shared,
  and its newest kept versions to restore, with ALL VERSIONS… for the rest. "Open in <app>" is left out until the
  page that frames a Craft web app is built (M5). SHARE… beside DOWNLOAD opens the share-link sheet.
- 2026-11-15: Web: the file browser follows the v2 Files frames (#24). A file's "…" menu has **Versions…**: the
  current file and the versions kept in `.craftcloud/versions/` (the newest 5), each to download or restore. A
  restore keeps the current file as a version first and writes with `If-Match`, so a change made meanwhile is never
  overwritten. The menu says "Move to trash" where it said "Delete". Uploading over a taken name, restoring from the
  trash with a new name, Recent's OPENED column and the phone layout already matched the frames.
- 2026-11-15: Desktop: a sign-in made with `craftcloud login` reaches the desktop client. With a server saved and
  no folder yet, the client reads the keychain at the start as when signed in (still on its own thread) and, with a
  sign-in, lists the cloud's folders and goes on to step 3 (refreshing an access token that ran out, once); without
  one it shows the Connect sheet with no alarm. A running client waiting for a sign-in hears of it over the control
  channel (`{"cmd": "signed_in"}`, sent by `login`), reads the settings again and takes it. Found testing 0.1.1 on
  Windows 11.
- 2026-11-15: Desktop: the window's place and size are never saved while it is minimised or hidden (eframe wrote
  Windows' 0×0 at (-32000, -32000), restored as 64×64 at the corner); the last good place is written back instead
  (`apps/craftcloud/src/geometry.rs`), and a window restored smaller than its minimum, or where Windows parks
  minimised windows, opens at its default size, centred. Found testing 0.1.1 on Windows 11.
- 2026-11-15: Desktop: a second start shows the running client's window on Windows and macOS as on Linux (the
  check is the same on every system; 0.1.1 didn't have it, and a minimised window didn't come back, see #2). The
  decision is one function, tested against a real control channel. Found testing 0.1.1 on Windows 11.
- 2026-11-15: Desktop: the tray's Show (and a second start, and a clicked notification) brings back a minimised
  window at once (#2). eframe runs only the app's `logic` while the window is minimised or hidden, and the tray
  was handled in `ui`; it is handled in `logic` now, with Quit and the close button. Found testing 0.1.1 on Windows 11.
- 2026-11-15: Build: the Craft web builds are fetched by `cargo xtask fetch-apps` and their pins updated by
  `cargo xtask bump-apps`, in Rust; the Python scripts `scripts/fetch-apps` and `scripts/bump-apps` are gone, and the
  image build no longer needs Python. The `-apps` image serves the same files and `apps.json`.
- 2026-11-15: Desktop: a long error detail in the Activity list is cut to its row (full text on hover) instead of
  spilling out of the panel over the time, badge and name, and "Syncing stopped" never ends a path with two full stops.
- 2026-11-15: Sync: a Windows computer leaves alone every cloud name Windows can't have (the names the other clients
  warn about: `< > : " | ? *`, a trailing dot or space, reserved names also with an extension), said once instead of
  failing each pass; cloud names that differ only in case are held there, all but the one it has. One rule in
  `core::names` (`Platform`) for the warning, the skip and the local path check. A failed download no longer leaves
  its part file, and part files older than a day are cleaned up. Found testing 0.1.1 on Windows 11.
- 2026-11-15: Windows: `craftcloud status` typed in Command Prompt or PowerShell prints again: `craftcloud.com`, a
  console program next to `craftcloud.exe`, runs it and passes its output, input and exit code through (#4). The
  MSI puts its folder on the user's PATH and, on uninstall only, removes the start-at-login value.
- 2026-11-15: Build: Rust is pinned to 1.100.0 in `rust-toolchain.toml`, the CI and release workflows and the Docker image, so
  local builds and CI use the same compiler (CI took the newest stable before, local builds whatever was installed).
- 2026-11-15: Version 0.1.1: stored HTML and SVG never run as pages, a file being saved is never overwritten by the
  sync, `sync --once` syncs a file saved just before it, and `craftcloud open` returns at once with a usual Windows path.
- 2026-11-15: CLI: `sync --once` run right after a file was saved no longer ends "up to date" without it: it
  waits the 2 s settle time once and syncs the file in a second pass, and lists what still waits as `waiting` (#15).
- 2026-11-15: Sync: a file being saved (written in the last 2 s) is held for the next pass instead of looking
  unchanged, and a download never replaces a local file that changed since the last sync (#36: a cloud change
  made while a file was being saved could replace the local edit with no conflict copy).
- 2026-11-14: CI: Claude checks every pull request and issue (`claude-pr.yml`, `claude-issues.yml`). A pull
  request gets a gitmoji in front of a title without one (so contributors needn't know gitmoji, and the merge
  commit still starts with one), labels, and one automated review comment that is updated on every push; an
  issue gets labels and no comment. The pull request's code is never checked out or run, Claude can only read,
  only inside the workspace, an answer that looks like a credential is never posted,
  and only gitmoji and labels from the lists in `.github/` get through (`.github/scripts/claude-bot.sh`, tested
  with hostile input in Packaging lint). Not run on GitHub yet: it needs the `CLAUDE_CODE_OAUTH_TOKEN` secret.
- 2026-11-14: Nix: a Home Manager module for the desktop client (`homeManagerModules.default`,
  `programs.craftcloud`: the package, start at login in the tray through the client's own autostart entry, the
  default for `craftcloud://` links) and a VM test of the server module (`checks.<system>.vm-sync`): a second
  machine signs in through the device flow and syncs a file each way. The server module gains `imageFile`, to load
  the image from the Nix store.
- 2026-11-14: Web: "Your share links" (`/links`, #19), opened from a new account menu under the person's name in
  the header (with the number of links) and from the share sheet's "See all your share links": every link with
  where its item is now (looked up by its id, as `craftcloud share ls`), when it expires, whether it has a
  password, how often it was opened and when it was made; filters All, Active, Expired and No longer there with
  counts, search, sort, COPY, and REVOKE after a confirmation (`DELETE /api/shares/{id}`; the item is never
  touched). A link whose item is in the trash or deleted shows the name it was shared with; a lookup that fails
  some other way says so and is not counted as gone. Built to the v2 Sharing frames; checked with egui_kittest
  tests and screenshots, not yet against a live server.
- 2026-11-14: Web: a "Share link" sheet in the file browser (#18), opened from a file's or folder's "…" menu
  ("Share link…") or the selection's SHARE: the link expires never, in 1, 7 or 30 days, or at the end of a day
  picked in a calendar (past days can't be picked), with an optional password (hidden unless SHOW, sent only in the
  request's body), then "Link ready" with the URL, COPY and a "Link copied" notice; a refusal says why in plain
  words and keeps the form. The expiry rules are the command line's `--expires`, now shared in `crates/core`
  (`shares::expiry_at`). Built to the v2 Sharing frames; checked with egui_kittest tests and screenshots, not yet
  against a live server.
- 2026-11-15: Single sign-on: the sign-in sheet follows the v2 frames: "Sign in with <button name>" spans the
  form with its label centred, the "or" rule sits closer to it, and "Signing you in…" has no empty action row.
  Web shots of every sign-in state (`login-sso*`).

- 2026-11-15: Single sign-on: **Admin › Server › Single sign-on** follows the v2 frames: "Asking <host> for its
  OpenID configuration…" and "Testing…" while Test connection runs, a failed test in plain words with the
  server's own words under it, and password sign-in stays on until Test connection has worked for the issuer URL.
  The three Off/On switches are separate widgets again, and the actions wrap on a phone. Web shots `admin-sso*`.
- 2026-11-14: Desktop on Linux (Hyprland and Sway with a bar's tray, checked with Wayle): desktop notifications for
  conflicts, a sign-in that ran out and a sync that can't go on (clicking one opens the window); starting the app
  again while it runs shows its window instead of a second copy with its own tray icon, and a second start at login
  ends quietly; the tray's icon and menu no longer change at every sync cycle (a tray host rebuilt an open menu each
  time); a removed device shows as signed out rather than as an error (OxiCloud answers its refresh with 404).
  The README's Hyprland and Sway notes say how to start it at login there.
- 2026-11-14: Testing: `cargo xtask e2e-sync`, a two-client sync smoke test against any server. Two scratch
  clients sign in as separate devices and sync a fresh folder pass by pass: create, edit, rename and move both
  ways, unicode names, a 20 MB file, 200 small files, deletes to the trash (checked through the REST API), a
  conflict copy on both sides and temporary-looking names (T-063, T-064). It checks that no other client syncs
  the account (through its sessions, for an admin account) and that the cloud and both clients end the same,
  keeps every pass's `--json` and ends with a PASS/FAIL table ([TESTING.md](TESTING.md)).
- 2026-11-15: Web: the public share page follows the v2 Sharing frames (#20): 40 px folder rows, names cut to fit
  on a phone, the Type fact names the format ("DOCX document"). A shared folder's subfolders open in place through
  OxiCloud's folder listing, which accepts the link's cookie; a folder lists up to 1000 items (five pages, then "The
  first 1000 items") and opens up to 32 levels down, never a folder already open above. A folder shows LOADING…
  until it is listed and an error when it can't be, never "empty". Names from the server are shown without control
  characters, a download is saved under the bare file name, and a token no link could have is "This link doesn't
  work" without asking the server.
- 2026-11-14: Releases: the Release workflow install-tests every Linux package before it makes a draft
  (`package-test`): the deb on Debian 12 and Ubuntu 22.04/24.04, the rpm on Fedora 41/42 (x86_64 and aarch64),
  the Arch package, the AppImage and the tarball, each installed with its package manager, checked, signed in and
  synced against a server built in the same run, upgraded from the previous release and removed. The script,
  `packaging/linux/test-package.sh`, also runs locally with Docker; RELEASING.md adds a manual desktop check on
  Fedora and Debian VMs before bigger releases.
- 2026-11-14: Version 0.1.0, the first release: the server image is published to `ghcr.io/jonasbove/craftcloud`
  (amd64 and arm64, slim and `-apps`) with provenance, and compose and the NixOS module default to it; the
  release no longer waits for the untried FreeBSD and riscv64 builds, which join the draft when they finish;
  `SECURITY.md` says how to report a vulnerability privately.
- 2026-11-14: Releases: the desktop client is packaged like the Craft apps (`packaging/`, `release.yml` replacing
  `desktop.yml`): AppImage with zsync, deb, rpm, Flatpak, Arch and tar.gz for Linux x86_64 and aarch64, an MSI and
  a portable zip for Windows x64 and arm64, a universal macOS DMG, and untried FreeBSD and riscv64 builds. A tag
  makes a draft release with checksums, provenance and, with a signing key, GPG signatures. One app id,
  `io.github.jonasbove.craftcloud`, everywhere; start-at-login entries move to it by themselves. Checked locally:
  the Linux x86_64 tarball, deb, rpm, AppImage and Arch package; the rest waits for the first CI run.
- 2026-11-14: Windows: the desktop client starts on a fresh Windows 11 (the C runtime is built into the exe). A first
  test on a Windows 11 VM: sign-in, the command line, two-way sync, the window, the tray and `craftcloud://` links
  work; the tray's Show doesn't bring back a minimized window, and idle CPU with the window open is to be
  measured on real hardware.
- 2026-11-14: Sync: a cloud folder whose listing failed is never trashed; folders named like temporary files sync,
  and the ignore rules apply to file names only, the same way on every side (T-062 to T-064; every computer must
  be updated together, see [RELEASE-NOTES.md](RELEASE-NOTES.md)). Future-dated files and long names sync; failed
  files show in the headline (T-059, T-060).
- 2026-11-14: Single sign-on: provider failures land on the sign-in sheet in plain words; [OIDC.md](OIDC.md) gains
  account matching, provider recipes and an end-to-end test with Keycloak. Nothing is tied to one installation:
  a generic default image and project links from `Cargo.toml`.
- 2026-11-14: UI pass 2 over the web frontend: v2 app cards, the phone layout of home and admin, the trash's
  DELETED column; kept versions follow renames and moves (T-049, T-050); a shared folder's subfolders open on the
  share page (T-015); search never stale across sessions (T-043).
- 2026-11-14: Command line, pass 2: every command has a summary and an example; `craftcloud://open/<path>` links;
  versions follow `mv`; `trash restore` onto a taken place is refused with `--rename` or `--as` (T-047); exit 9
  while the sync folder is missing or unreadable (T-033, T-053).
- 2026-11-14: Sync, from testers' reports: a renamed or moved folder is one move (T-022); a deleted folder is one
  trash item (T-032); unreadable places are unknown, never deleted (T-033); a file and a folder of one name both
  survive (T-029); a vanished sync folder is never made again on its own (T-034); interrupted downloads resume
  (T-018); a silent connection goes offline and recovers (T-037); unchanged bytes aren't uploaded again (T-035);
  editor lock and swap files never sync (T-028); names the cloud can't take are never sent (T-027).
- 2026-11-14: UI pass 1: the launcher look over the web and desktop screens ([design-notes/pass-1.md](design-notes/pass-1.md)).
- 2026-11-14: Craft apps: `craftcloud apps`, `files --app` and `save` (the integration contract) and
  `craftcloud://` deep links on Linux and Windows ([CRAFT-INTEGRATION.md](CRAFT-INTEGRATION.md)); a design for
  files on demand ([FILES-ON-DEMAND.md](FILES-ON-DEMAND.md)).
- 2026-11-14: Web file browser: upload (button and drag and drop), new folder, rename, move; the public share page
  `/s/<token>` (T-010). Desktop: a second pass over the screens; per-file sync status and the tray's recent files.
- 2026-11-13: Web: a Files page with Recent, Starred, Trash and search; Admin › Users and Admin › Server; single
  sign-on with an OpenID Connect provider. Command line: `craftcloud <command>` with the desktop client's sign-in,
  headless sync (`sync --once`, `daemon`), `users add --admin`.
- 2026-11-13: Desktop: every screen rebuilt from the v2 design; renames and moves are moves on both sides (T-003);
  deletes go to the cloud's trash; new folders join the sync (T-001). Core: OxiCloud's file API (folders, files,
  batch, trash, favourites, recent, search, shares, versions).
- 2026-11-13: CraftCloud 0.1.0: the server image, the desktop client with device sign-in and the two-way sync
  folder, the web frontend's setup, sign-in, device approval and home pages, the NixOS module and the design
  (`design/`). The history before this is squashed into that one commit.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-14 | major | First version: the README's roadmap moves here, rebuilt from the docs and the git history. |
| 2026-11-15 | minor | M8, a native NixOS deployment, with the Craft web apps built by Nix from `apps.toml`. |
| 2026-11-16 | minor | M9, any WebDAV server as the storage; the Upcoming list put back in order after a merge, without the Windows command line, which is done. |
| 2026-11-16 | minor | Version 0.2.0; M4 done; M9 planned for 0.3.0. |
| 2026-11-17 | minor | M10, no Python in the tree. |
| 2026-11-17 | minor | M9's design, with the decisions; on `main` behind an opt-in until 0.3.0. |
| 2026-11-17 | minor | M9 pull request 1: the sync engine behind a storage trait, OxiCloud its first driver; no change for a person. |
| 2026-11-17 | minor | M9 pull request 2: sync modes, and the plan cut to download only; no change for a person. |
| 2026-11-18 | minor | M9 pull request 3: the server check (the probe in core, its runner, `craftcloud server check`); OxiCloud syncs as before. |
| 2026-11-18 | minor | M9 pull request 4a: the generic WebDAV driver, `Credential`, `login --kind webdav`, exit code 10, behind the `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` opt-in; OxiCloud syncs as before. |
| 2026-11-18 | minor | M9 pull request 5: the live suite against Nextcloud, ownCloud, OpenCloud, `rclone serve webdav` and Apache `mod_dav` (`CRAFTCLOUD_TEST_KIND`, `tests/other-servers/`); the OxiCloud suite unchanged. |
| 2026-11-18 | minor | M9 pull request 6: the Nextcloud profile (Login Flow v2, its trash and versions after the check's round trips, the checksum check on every write, Q13), behind the opt-in; OxiCloud syncs as before. |
