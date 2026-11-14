# Release notes

## Unreleased

- **Server: files opened in the browser stay files.** HTML, SVG and XML that people store are served with a
  sandbox policy, so they never run as pages on your CraftCloud's address. WebDAV names with a backslash are refused
  (as the desktop client already does).
- **Building the server image yourself on an arm64 machine works** (a Raspberry Pi, an Apple Silicon Mac):
  `docker compose build` stopped in the web frontend step there. The published images were not affected.

## 0.1.0

The first release of CraftCloud: a self-hosted cloud for the Craft apps, running OxiCloud 0.10.0. It is an
**early alpha**: back up what matters, and read the known limits below.

**What's in it**

- **The server**, as an image for amd64 and arm64: `ghcr.io/jonasbove/craftcloud:0.1.0`, and
  `:0.1.0-apps` with the Craft apps for the browser. The unmodified OxiCloud 0.10.0 server behind Caddy, with
  CraftCloud's own web frontend: first-run setup, files (upload, folders, rename, move, trash, search, starred,
  recent, versions), the page a share link opens, and admin for users and the server. Share links are made
  from the command line for now. Single sign-on with any OpenID
  Connect provider is optional ([OIDC.md](https://github.com/jonasbove/craftcloud/blob/main/docs/OIDC.md)).
- **The desktop client** for Linux, Windows and macOS, which is also the `craftcloud` command line: a sync
  folder that keeps chosen cloud folders in step both ways, with conflict copies, the last 5 versions of
  every file, deletes that go to the cloud trash, a hold on mass deletes, and sign-in through the browser.
  `craftcloud://` links open synced files in the Craft apps.
- **Packages:** AppImage, deb, rpm, Flatpak, Arch and tar.gz for Linux (x86_64 and aarch64); an MSI installer
  and a portable zip for Windows (x64 and arm64); one DMG for both kinds of Mac. A Nix flake with a NixOS module
  for the server.

**Known limits** (more in [STATUS.md](https://github.com/jonasbove/craftcloud/blob/main/docs/STATUS.md))

- The desktop builds are not code-signed: macOS asks once (right-click › Open) and Windows SmartScreen asks once
  (More info › Run anyway).
- **Every computer syncing an account must run the same version** (see "Upgrade all devices together" below).
- Windows: the command line prints nothing in a terminal yet, and the tray's Show doesn't bring back a minimized
  window (click the taskbar button).
- `craftcloud sync --once` leaves a file written in the last 2 seconds for the next pass, but still reports
  "up to date": run it again in a script that has just written files. *(Added after the release.)*
- Windows: `craftcloud open` shows paths with a `\\?\` prefix, and a script that captures its output waits until
  the opened app closes. *(Added after the release.)*
- macOS: a `craftcloud://` link starts CraftCloud but doesn't open the file yet.
- The Flatpak can't turn on start at login.

**If you ran a build from before this release**, the notes below say what changes.

### Desktop: one app id, `io.github.jonasbove.craftcloud`, and new packages

The desktop client now has one id everywhere: the window, the Linux menu entry and icons, the macOS app bundle and
the start-at-login entries. Releases come as the Craft apps' do: AppImage, deb, rpm, Flatpak, Arch and tar.gz for
Linux; an MSI installer and a portable zip for Windows (x64 and arm64); one DMG for both kinds of Mac.

- **Start at login moves by itself.** If you turned it on in an earlier version, it stays on: the old entry
  (`~/.config/autostart/craftcloud.desktop` on Linux, `~/Library/LaunchAgents/com.craftcloud.client.plist` on
  macOS) still counts, and the next time you change the setting it is replaced by
  `io.github.jonasbove.craftcloud.desktop` / `.plist`. Windows is unchanged.
- **Linux: the menu entry is renamed** to `io.github.jonasbove.craftcloud.desktop`, with icons of the same name.
  If you installed from the tarball, run the new `install.sh`: it removes the old `craftcloud.desktop` and
  `craftcloud.png` icons from `~/.local/share` (`./install.sh --remove` does too). A deb, rpm or Arch package
  can't remove files in your home folder, so if you used `install.sh` before and now install a package, run the
  old or new `./install.sh --remove` once, or CraftCloud is listed twice.
- **macOS: the bundle id is now `io.github.jonasbove.craftcloud`** (it was `com.craftcloud.client`). Replace
  the old `CraftCloud.app` in Applications with the new one. Your sign-in, settings and sync folder are kept;
  macOS may ask once whether CraftCloud may use its saved sign-in in the keychain (choose **Always Allow**).
- **Windows: the program is now `craftcloud.exe`**, installed by the MSI into
  `%LOCALAPPDATA%\Programs\CraftCloud`. If you ran `CraftCloud.exe` from the old zip with start at login on,
  turn the setting off and on again once in the new one, so it starts the installed copy.

### Windows: the client starts on a fresh Windows

`CraftCloud.exe` now has the C runtime built in. Before, it needed the Visual C++ runtime
(`VCRUNTIME140.dll`) and, on a Windows without it, closed at once without a message.

### Upgrade all devices together (sync ignore rules)

The rules for what the sync leaves alone changed: the editor temporary-file rules (`*.tmp`, `*~`,
`*.part`, `#*#`, `.#*`, `~$*`, `*.swp`) now apply to **file names only**, so a *folder* such as
`projekt.tmp/` or `Backup~/` syncs like any other. Every client now applies one rule to its own
folder, to the cloud listing and to its saved state alike, so a name it leaves alone is never
downloaded, uploaded, or taken as deleted.

**Clients from before this change apply the old rules and can't be fixed from here.** An old client
syncing the same account would download such a folder that a new client uploaded, then see it as
deleted on its side and move it to the cloud trash, and the other computers would follow. Update
every computer that syncs the account (desktop clients and `craftcloud daemon` alike) before
creating folders with such names. Anything moved to the trash this way can be restored from it.
