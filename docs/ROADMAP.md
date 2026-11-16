# CraftCloud roadmap

**Stage: early alpha** · version 0.1.1 (fixes for 0.1.0, the first release: a published image and desktop builds)

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-16 · **Change:** minor (property tests of the sync
> decisions)

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
| Desktop client (tray, Activity, Folders, Settings, start at login) | done | [README](../README.md#downloads) |
| Web file browser (upload, download, rename, move, delete, trash, versions, details, recent, starred, search) | done | [design-notes/pass-2.md](design-notes/pass-2.md) |
| Share links | done: public share page, CLI, the web's "Share link" sheet and "Your share links" page | [CLI.md › Share links](CLI.md#share-links) |
| Admin (users, server settings, Craft web apps on or off) | done, on the web and in the CLI | [CLI.md](CLI.md#server-administration-owners-and-admins) |
| Command line | done; on Windows through `craftcloud.com` (untried on Windows) | [CLI.md](CLI.md) |
| Craft apps through the sync folder | done (every app) | [CRAFT-INTEGRATION.md § 1](CRAFT-INTEGRATION.md#1-the-sync-folder-works-today-every-app) |
| `craftcloud://` deep links | done on Linux and Windows; macOS needs an Apple Event handler | [CRAFT-INTEGRATION.md § 2](CRAFT-INTEGRATION.md#2-deep-links-craftcloud) |
| Integration contract (`craftcloud apps`, `files`, `save`) | done | [CRAFT-INTEGRATION.md § 3](CRAFT-INTEGRATION.md#3-the-integration-contract-the-craftcloud-command) |
| Craft web apps in the browser | in part: the `-apps` image serves and gates them; opening a cloud file in one and saving it back is not built | [CRAFT-INTEGRATION.md § 6](CRAFT-INTEGRATION.md#6-the-craft-web-apps-in-the-browser) |
| "Open from / Save to CraftCloud" in the launcher and the apps | proposal, upstream | [CRAFT-INTEGRATION.md § 4–5](CRAFT-INTEGRATION.md#4-the-artcraft-launcher-proposal-upstream) |
| Files on demand | design only | [FILES-ON-DEMAND.md](FILES-ON-DEMAND.md) |
| Releases (published image, desktop release, signing) | desktop packaging done, not yet run in CI: the Craft apps' release file set (AppImage, deb, rpm, Flatpak, Arch, tar.gz; MSI and portable zip; universal DMG) from `release.yml` as a draft release, GPG-signed when a key is set, and only after every Linux package passed its install test (`package-test`); no published image, no code signing | [README](../README.md#downloads) |
| Upstream gaps and proposals | drafted, not filed | [UPSTREAM.md](UPSTREAM.md) |

## Milestones

| Milestone | Goal | Where it stands (2026-11-14) |
|---|---|---|
| M0 Server image | OxiCloud unmodified behind Caddy, CraftCloud's settings service, slim and `-apps` images, NixOS module | Done |
| M1 Sync client | Device sign-in, a two-way sync folder with guarded writes, versions and conflict copies, tray | Done; hardening continues from testers' reports |
| M2 Web frontend | Setup, sign-in, device approval, home, the file browser, admin, single sign-on | Done, except the share-link sheet |
| M3 Command line and integration | `craftcloud <command>` for files, sync, sharing, versions and admin; the contract the Craft apps and the launcher can use | Done; the command line prints in Windows terminals too (tried on Windows 11) |
| M4 First release | Published images, a tagged desktop release, release notes | Not started |
| M5 Craft web apps in the browser | Open a cloud file in its Craft web app from the web file browser and save it back (`craft-web-host`, the shim for the others) | Serving and gating done; the editor page and the shim not built |
| M6 Files on demand | Placeholders that download when opened (Windows, Linux, macOS) | Design only |
| M7 Signed desktop builds | Signed and notarized macOS builds, signed Windows builds | Not started; v1 is unsigned |
| M8 Native NixOS deployment | OxiCloud, Caddy and craftcloud-admin as NixOS services without Docker, with the Craft web apps built by Nix from `apps.toml` | Not started; the NixOS module runs the Docker image today |
| M9 Any WebDAV server as the storage | The desktop client and command line sync with any WebDAV server (Nextcloud, ownCloud, a NAS, `rclone serve webdav` …), not only OxiCloud: a check of the server on connect, and the client's own trash and versions where the server has none | Planned |

## Current focus

Sync correctness from testers' reports (the T-numbers in the commit log: renames, folder deletes, unreadable
places, names the cloud can't take, interrupted downloads, quota) and the second UI pass over the web frontend
([design-notes/pass-2.md](design-notes/pass-2.md)).

## Upcoming

In the order planned so far; detail in [STATUS.md](STATUS.md#known-limits) and the docs linked.

1. **A first release:** a published image (slim and `-apps`), a first run of the **Release** workflow
   (`release.yml`) on a tag, a release signing key (`RELEASE_SIGNING_KEY`), and the AUR recipes in
   `packaging/arch/`, with [RELEASE-NOTES.md](RELEASE-NOTES.md).
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

## Progress log

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
