# Status

What works today and how it was checked, and the known limits. What comes next is in [ROADMAP.md](ROADMAP.md).

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-14 · **Change:** major (first version, from `main` at
> `1a37a72`; updated for 0.2.0)

Everything is checked against OxiCloud **0.10.0**, the pinned server. `cargo xtask ci` (fmt, clippy with warnings
denied, the workspace tests and the wasm build) is the gate, and GitHub Actions runs it, plus the Trunk web build,
on Linux for every push to `main` and every pull request. The live tests need a throwaway server from `scripts/smoke-test` and are skipped without one; CI's `live` job builds the image from the tree and runs them (and `cli_live`) against it on the same pushes and pull requests.
What no test reaches, from a coverage run, is in [TESTING.md › Coverage](TESTING.md#coverage).

## What works, and how it was checked

| Area | What works | Checked by |
|---|---|---|
| Server image | OxiCloud unmodified behind Caddy; `craftcloud-admin` serves the settings and gates each `/apps/*` file; slim and `-apps` images; settings in `settings.env` on the data volume | `scripts/smoke-test` starts the image with PostgreSQL and checks its routes (the frontend at `/` and `/device`, OxiCloud's own pages and CalDAV/CardDAV blocked, the API passed through, wasm served precompressed); unit tests of the gate and the settings in `apps/craftcloud-admin` |
| First-run setup | Five steps in the browser, or `scripts/create-owner` in the terminal; `scripts/add-user` after that | The setup pages' UI tests (egui_kittest) in `crates/ui-egui/src/web/tests.rs`; API answers recorded from OxiCloud 0.10.0 in `crates/core/tests/fixtures/oxicloud-0.10.0/`; the scripts (sh and curl, no Python) against a real server in `the_operator_scripts` (`apps/craftcloud/tests/cli_live.rs`, CI's `live` job) |
| Desktop sign-in | OAuth 2.0 device flow: the client shows a code, the person approves it on the server's `/device` page, the tokens go to the system keychain and are refreshed; a sign-in made with `craftcloud login` is taken by the desktop client, at its start or while it waits for one; signing out (`craftcloud logout`, Settings › Sign out) ends the session on the server too, and says so when the server can't be told | Recorded server answers in `crates/core`; the live sync tests sign in against a real server |
| Single sign-on | Any OpenID Connect provider, set up in **Admin › Server › Single sign-on** (Test connection in plain words; password sign-in goes off only after it worked); failures land on the sign-in sheet in plain words | End to end on 2026-11-14 with Keycloak 26.0 and headless Chromium; untested with the other providers ([OIDC.md › What was tested](OIDC.md#what-was-tested)) |
| Sync | Two-way sync of the chosen folders; guarded writes (`If-None-Match`, `If-Match`); the last 5 versions of every overwrite; conflict copies; renames and moves as moves; folder deletes as one trash item; mass deletes held for a decision; interrupted downloads resumed; names the cloud can't take never sent ([ARCHITECTURE.md › Sync](ARCHITECTURE.md#sync)) | Unit tests of the decision table (`core::reconcile`), and property tests that check its rules against generated states, scans and listings; `crates/native/tests/live.rs` runs two computers against a real server (renames, folder moves, deletes, quota, resumed downloads, a silent connection, offline edits); the fixes for testers' reports (T-001 to T-064) come with tests; `cargo xtask e2e-sync` runs two separate clients through a real server, case by case, also as a mixed pair (0.1.1 beside this tree on Linux, both ways round: versions, conflicts, folder deletes and leftovers pass) ([TESTING.md](TESTING.md)) |
| Desktop client | Tray (sync now, pause, open the folder or the web page), Activity, Folders and Settings tabs, start at login, Chinese, Japanese and Korean names; one copy runs (starting it again shows its window); notifications on Linux for conflicts, a sign-in that ran out and a sync that can't go on | egui_kittest tests in `crates/ui-egui/src/desktop/tests.rs`; screenshots of every scene with `--features shots` ([design-notes/pass-1.md](design-notes/pass-1.md)); unit tests of when the tray changes, of its menu items against the v2 tray frame, and of what is notified (`apps/craftcloud/src/tray.rs`, `notify.rs`) and of the second start (`control.rs`). On 2026-11-14, on Linux in a headless Sway session with Wayle as the tray host and notification service, against a test server: the icon, a left click, the menu's Show and Quit, a `--hidden` start (no window, the icon also when the bar starts later), closing the window with the compositor while the sync carries on, starting it again, `craftcloud://open` and `reveal` from `xdg-open`, and the notifications |
| Web frontend | Home with the steps to connect a computer, sign-in, device approval, the file browser (upload, download, new folder, rename, move, delete, trash, recent, starred, search; a file's version history to download or restore a kept version; a details panel beside the list for the one file selected, with its kind, the app that opens it, DOWNLOAD and SHARE…, where it is and its last versions), the "Share link" sheet (from a file's or folder's menu or the selection's SHARE: expiry Never, 1, 7 or 30 days or to the end of a picked day, an optional password, then the link with COPY), "Your share links" (`/links`, from the account menu under the person's name: every link with where its item is now, its expiry, password, opens and date, filters, search, sort, COPY and REVOKE with a confirmation), the public share page `/s/<token>` (a file, a folder and its subfolders, a password, an expired or removed link; up to 1000 items a folder, 32 levels below the shared one) | egui_kittest tests in `crates/ui-egui/src/web/` (the share sheet's request bodies for every expiry, the hidden password, the link and COPY, the server's refusal; the links page's filters, search, sort, an item gone or not looked up, revoke and cancel, the empty page, the account menu); screenshots of every page at desktop and phone width ([design-notes/pass-2.md](design-notes/pass-2.md)); `cargo xtask e2e-web` runs the app in headless Chromium against a fresh server: setup, sign-out and sign-in (the session cookie HttpOnly, Secure only on https), an upload, a share link made in the sheet and opened and downloaded signed out, a device approval on `/device`, a new user on Admin › Users with the CSRF header ([TESTING.md](TESTING.md)); the links page not yet against a live server |
| Admin | **Admin › Users** and **Admin › Server** (single sign-on, the Craft web apps on or off, app by app), only for owners and admins; the same in `craftcloud server …` | `crates/ui-egui/src/web/admin/tests.rs`; the CLI end-to-end test; `cargo xtask e2e-web` makes a user in the browser and checks that cookie-session admin writes without the CSRF header are refused |
| Command line | `craftcloud <command>`: files, sync, sharing, versions, trash, search and admin; `server check` says what the server guarantees the sync and the mode that allows (known for CraftCloud's server, not measured); `--json` everywhere, stable exit codes ([CLI.md](CLI.md)) | Argument parsing and output tests in `apps/craftcloud`; `apps/craftcloud/tests/cli_live.rs` runs every command against a real server (for `server check`: kind, mode, and that no probe folder was written) |
| Craft apps | Every app works through the sync folder; the integration contract (`craftcloud apps`, `files --app`, `save`); `craftcloud://open` and `reveal` links ([CRAFT-INTEGRATION.md](CRAFT-INTEGRATION.md)) | Unit tests in `crates/core/src/catalog.rs` (checked against `apps.toml`), `deeplink.rs`, `crates/native/src/open.rs`, `local_files.rs` and `control.rs` |
| Nix | A flake with the desktop client, a NixOS module for the server and a Home Manager module for the client (`programs.craftcloud`: the package, start at login with `--hidden`, the `craftcloud://` default) | `checks.<system>.vm-sync` (`nix/vm-test.nix`): the server module runs the released 0.1.0 image in a VM with NixOS's PostgreSQL, and a second VM signs the command line in through the device flow (approved over the API) and syncs a file each way with `sync --once`; `checks.<system>.home-manager` evaluates the Home Manager module; `checks.<system>.module` evaluates a system with the server module. Run by hand on x86_64; not in CI |
| Desktop packages | `packaging/` and `release.yml`: Linux AppImage (+ zsync), deb, rpm, Flatpak, Arch and tar.gz (x86_64, aarch64); Windows MSI and portable zip (x64, arm64); a universal macOS DMG; FreeBSD and riscv64 attempted; a draft release with checksums, provenance and optional GPG signatures | On 2026-11-14, locally on Linux x86_64 only: the tarball, deb (`dpkg-deb`), rpm (`rpm -qip`/`-qlp`), AppImage (`--version`, its menu entry), Arch packages (`makepkg`, also as `release.yml` runs it in an `archlinux:base-devel` container), `install.sh` in a scratch home, `desktop-file-validate` and `appstreamcli validate` (also with Ubuntu 22.04's AppStream 0.15), `actionlint` and the release job's signing steps with a test key. Flatpak, macOS, Windows, aarch64, FreeBSD and riscv64 have not run yet. *Install tests in CI (new, not run on GitHub yet):* on every Release run `package-test` (no draft unless it passes) checks that each `.deb` (Debian 12, Ubuntu 22.04, 24.04) and `.rpm` (Fedora 41, 42) on x86_64 and aarch64, the Arch package (x86_64) and the AppImage and tarball (Ubuntu 22.04, x86_64) install with the distro's package manager, print the right `--version`, have a valid desktop entry, icons, AppStream file and `craftcloud://` handler, sign in through the device flow without a browser or keychain and sync both ways (`put`/`ls`/`get`, `sync --once`) against a server image built from the same commit, upgrade from the previous published release, and remove cleanly (`packaging/linux/test-package.sh`). On 2026-11-14 that script ran locally on x86_64 for the deb (Debian 12, Ubuntu 24.04), rpm (Fedora 42), AppImage and tarball (Ubuntu 22.04), with the round trip, and the upgrade from a 0.0.9 build of the same tree (Debian 12, Fedora 42); the CI job itself and aarch64 have not run yet. Not covered by containers: the tray, the keychain, browser sign-in and start at login (a manual check before bigger releases, [RELEASING.md](RELEASING.md)) |

## Known limits

- **Start at login doesn't work from the Flatpak.** The entry is written inside the sandbox, which the session
  never reads; use the AppImage, deb, rpm or Arch package for that.
- **Desktop builds are unsigned.** macOS asks once (right-click › Open, or remove the quarantine attribute) and
  Windows SmartScreen asks once (More info › Run anyway). Signing is not planned for v1.
- **macOS deep links don't open the file.** macOS hands a `craftcloud://` link to the app as an Apple Event
  (`kAEGetURL`), and the client has no handler for it (winit doesn't expose one). A click starts or focuses
  CraftCloud; `craftcloud 'craftcloud://…'` from a terminal works.
- **The Windows command line in a terminal: Ctrl+C is unconfirmed.** `craftcloud.com` (`apps/craftcloud-cli`) runs
  `craftcloud.exe` for a terminal and passes its output through ([CLI.md › Windows](CLI.md#windows)); the MSI puts
  its folder on the user's PATH and removes the start-at-login value on uninstall. On a Windows 11 VM (2026-11-15,
  an MSI built from these changes) the output, the exit codes, the PATH entry and the uninstall's cleanup worked.
- **Windows: the tray's Show doesn't bring back a minimized window.** Seen on a Windows 11 VM; click the taskbar
  button instead.
- **Idle CPU on Windows is still to be re-checked.** An open, idle window drew 4 frames a second, which took
  about 3.6 cores on a Windows 11 VM drawing in software (#3). It now draws only when something changes and once a
  minute otherwise; on Linux with software rendering that took idle CPU from 22 % to 1 % of a core. A window
  hidden in the tray kept drawing a frame at every tick on Windows (about 1.5 cores on the VM); it now draws
  nothing while hidden. Neither is measured on Windows again yet.
- **Names Windows can't have don't sync to a Windows computer.** A cloud name with `< > : " | ? *`, a trailing dot
  or space, or a reserved name (`CON`, `NUL.txt`, `COM1.log`, also with an extension) stays in the cloud there and
  is reported once; Linux and macOS sync it and warn. Cloud names that differ only in case are held on Windows
  except the one the computer already has. macOS is usually case-insensitive too, but has no such check yet: there
  the download guard keeps the second name from replacing the first, and it is retried each pass. Checked by unit
  tests on Linux with the Windows rules (`names::Platform`); not yet on Windows itself.
- **A file saved in the last two seconds waits for the next pass** (an app may still be writing it).
  `craftcloud sync --once` waits that out once and syncs it in a second pass; a file still being written after
  that is listed as `waiting` (and in the headline), and the exit code stays 0.
- **Files on demand is a design only.** v1 is a full sync folder: every file in the chosen folders is downloaded
  ([FILES-ON-DEMAND.md](FILES-ON-DEMAND.md)).
- **The Craft web apps can't open cloud files yet.** The `-apps` image serves the web builds and switches them on
  and off per app, but the page that frames an app and passes files through `craft-web-host`, and the shim for the
  apps without it, are not built ([CRAFT-INTEGRATION.md § 6](CRAFT-INTEGRATION.md#6-the-craft-web-apps-in-the-browser)).
- **"Open from / Save to CraftCloud"** inside the launcher and the apps are proposals to their authors
  ([CRAFT-INTEGRATION.md § 4–5](CRAFT-INTEGRATION.md#4-the-artcraft-launcher-proposal-upstream)).
- **Update every computer together** after the sync ignore-rule change: an older client can move a folder named
  like a temporary file to the cloud trash ([RELEASE-NOTES.md](RELEASE-NOTES.md)).
- **Server gaps** CraftCloud works around (OxiCloud 0.10.0): no server-side versions, so the client and the web
  frontend keep them in `.craftcloud/versions/`, by path (WebDAV has no file id); folder ETags are not
  reliable, so every folder is listed each cycle; device tokens have the user's full rights (the scope isn't
  enforced). The full list is in [UPSTREAM.md](UPSTREAM.md).

## Next

See [ROADMAP.md](ROADMAP.md#upcoming) for the ordered plan.
