# CraftCloud roadmap

**Stage: early alpha** · version 0.1.0, the first release (a published image and desktop builds)

> **Last reviewed:** 2026-11-14 · **Last updated:** 2026-11-14 · **Change:** major (first version, built from the
> README's roadmap, the docs and the git history up to `1a37a72`)

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
| Share links | public share page and CLI done; no "create a share link" sheet in the web file browser | [CLI.md › Share links](CLI.md#share-links) |
| Admin (users, server settings, Craft web apps on or off) | done, on the web and in the CLI | [CLI.md](CLI.md#server-administration-owners-and-admins) |
| Command line | done; no console output from Windows release builds | [CLI.md](CLI.md) |
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
| M3 Command line and integration | `craftcloud <command>` for files, sync, sharing, versions and admin; the contract the Craft apps and the launcher can use | Done, except console output on Windows |
| M4 First release | Published images, a tagged desktop release, release notes | Not started |
| M5 Craft web apps in the browser | Open a cloud file in its Craft web app from the web file browser and save it back (`craft-web-host`, the shim for the others) | Serving and gating done; the editor page and the shim not built |
| M6 Files on demand | Placeholders that download when opened (Windows, Linux, macOS) | Design only |
| M7 Signed desktop builds | Signed and notarized macOS builds, signed Windows builds | Not started; v1 is unsigned |

## Current focus

Sync correctness from testers' reports (the T-numbers in the commit log: renames, folder deletes, unreadable
places, names the cloud can't take, interrupted downloads, quota) and the second UI pass over the web frontend
([design-notes/pass-2.md](design-notes/pass-2.md)).

## Upcoming

In the order planned so far; detail in [STATUS.md](STATUS.md#known-limits) and the docs linked.

1. **A first release:** a published image (slim and `-apps`), a first run of the **Release** workflow
   (`release.yml`) on a tag, a release signing key (`RELEASE_SIGNING_KEY`), and the AUR recipes in
   `packaging/arch/`, with [RELEASE-NOTES.md](RELEASE-NOTES.md).
2. **Share links on the web:** a sheet in the web file browser to create, copy and remove a link (the API, the CLI
   and the public page exist).
3. **Craft web apps in the browser:** the page that frames an app and talks `craft-web-host`, and the shim for the
   apps without it ([CRAFT-INTEGRATION.md § 6](CRAFT-INTEGRATION.md#6-the-craft-web-apps-in-the-browser)).
4. **macOS deep links:** an Apple Event (`kAEGetURL`) handler, so a `craftcloud://` click opens the file.
5. **The command line on Windows:** a console build, so release builds print to the terminal.
6. **Files on demand,** in the stages of [FILES-ON-DEMAND.md § 6](FILES-ON-DEMAND.md#6-staged-plan).
7. **Upstream:** the drafts in [UPSTREAM.md](UPSTREAM.md) and server-side versioning for OxiCloud, filed only after
   they are agreed.
8. **Unmaintained D-Bus crates:** drop the `deny.toml` ignores for zvariant, zvariant_derive and zbus_names
   (RUSTSEC-2026-0370 to 0372) once egui (accesskit) and keyring move to zbus 6.

## Progress log

- 2026-11-15: Web: the file browser's **details panel** (#24). On a wide page, the one file selected shows beside the
  list: a preview (the image, or the mark of the Craft app that opens it), its name and star, "DOCX document · 248 KB
  · opens in WordCraft", DOWNLOAD and the "…" menu, when it changed, where it is, its owner and whether it is shared,
  and its newest kept versions to restore, with ALL VERSIONS… for the rest. "Open in <app>" is left out until the
  page that frames a Craft web app is built (M5), and "Share…" comes with the share-link sheet.
- 2026-11-15: Web: the file browser follows the v2 Files frames (#24). A file's "…" menu has **Versions…**: the
  current file and the versions kept in `.craftcloud/versions/` (the newest 5), each to download or restore. A
  restore keeps the current file as a version first and writes with `If-Match`, so a change made meanwhile is never
  overwritten. The menu says "Move to trash" where it said "Delete". Uploading over a taken name, restoring from the
  trash with a new name, Recent's OPENED column and the phone layout already matched the frames.
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
