# Release notes

## Unreleased

- **Nextcloud, experimental and off.** With `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`, `craftcloud login cloud.example.org
  --kind nextcloud` signs in to a Nextcloud through the browser the way its own clients do (an app password is made
  for this computer; your account password never reaches CraftCloud; `--user NAME` takes an app password you made
  in Settings › Security instead), and the sync uses Nextcloud's own trash and versions once the check on connect
  has confirmed them: a delete lands in Nextcloud's trash bin, `trash ls|restore` and `versions` work on them, and
  `trash rm|empty` answer "not available on this server" (CraftCloud never purges a person's trash). Nextcloud
  reuses a file's version tag when it is overwritten within the same second as the write before it, so every
  upload carries its checksum and is read back; a write by someone else in that second is kept as a conflict copy,
  never overwritten. `logout` deletes the app password on the server. **A new kind of server: upgrade all devices
  together.** Every device that syncs an account on a Nextcloud must run this version or later: no earlier
  CraftCloud can sync with one (a generic WebDAV sign-in to a Nextcloud, made with an earlier build of this opt-in,
  keeps its client trash in `.craftcloud/trash/`; sign in again with `--kind nextcloud`).
- **Other WebDAV servers, experimental and off.** The command line can sign in to any other WebDAV server
  (`craftcloud login <WebDAV address> --kind webdav --user NAME`, with an app or WebDAV password) and sync with it,
  keeping its own trash and versions in `.craftcloud/` there; the check on connect decides two-way or download
  only. On such a server the client trash and version commands work too (`craftcloud trash ls|restore|rm|empty`,
  `craftcloud versions`), the client trash keeps an entry 30 days (by the server's clock; only `trash empty` clears
  it sooner), the check re-runs by itself when it is more than a day old or the server's version string changed, and
  the sync drops to download only (and says so) if the server ever replaces a file it was told to leave alone. All
  of it is switched off unless the environment variable `CRAFTCLOUD_EXPERIMENTAL_WEBDAV` is exactly `1`
  (nothing in the release, its packaging or its settings sets it): without it, `--kind` isn't offered, and a
  settings file naming another kind of server is refused with a message, nothing synced. CraftCloud's own server
  syncs exactly as before. **A new kind of server: upgrade all devices together.** Every device that syncs an
  account on another WebDAV server must run this version or later: no earlier CraftCloud can sync with such a
  server, so none can undo its plan, and from now on any change to how a server kind is decided or what it skips
  gets its own line here, as for CraftCloud's server.
- **A folder another sync program syncs is refused.** The desktop client, `craftcloud daemon` and `sync --once` no
  longer sync a folder that holds another sync client's database (`.sync_*.db`, `.owncloudsync.log` or
  `.csync_journal.db`, Nextcloud's or ownCloud's desktop client): two sync engines on one folder fight over it.
  Choose another folder. Nothing else needs care when updating.
- **`craftcloud status --json` says the sync `mode`** (`two_way` on a CraftCloud server), `sync --once --json` too,
  and a new exit code 10 (kind `unsupported`) means "not available on this server". Existing codes and fields keep
  their meaning.
- **`craftcloud server check`** prints what the server guarantees the sync, each check in plain words, and the sync
  mode that allows. On a CraftCloud server it reports the known capabilities (two-way) and writes nothing to the
  server. Nothing needs care when updating.
- **Web: the right version before the server answers.** A page drawn before the server's settings arrived showed
  version 0.1.0; it now shows the version it was built as.
- **Server scripts no longer need Python.** `scripts/create-owner`, `scripts/add-user` and `scripts/app-password`
  (and `scripts/smoke-test`) run with only sh and curl. They ask, print and exit as before; passwords no longer
  show in a command line while they run.

## 0.2.0

**Update the server and every desktop client, all of them together:** three sync fixes change how a client decides
(a file moved onto one renamed elsewhere, a cloud file listed without a version, and on Windows the names Windows
can't have), and an older client syncing the same account can still make the old plan. Read "When updating" first;
nothing else in your settings or data needs changing.

### When updating

- **Sync on Windows: names Windows can't have are left alone. Upgrade all devices together.** A Windows computer
  no longer tries to download a cloud file or folder whose name Windows can't have (`< > : " | ? *`, a name that
  ends in a dot or a space, or a reserved name such as `CON`, `NUL.txt` or `COM1.log`): it stays in the cloud, is
  never taken as deleted, and the activity list says so once instead of failing on every pass. Linux and macOS keep
  syncing these names and warn, as before. Two cloud names that differ only in upper and lower case (`Report.txt`,
  `report.txt`) can't both live in one Windows folder: the one the computer already has keeps syncing and the
  other is held in the cloud until one is renamed. A `CON.txt` an older Windows client made stays on that computer
  but no longer syncs; rename it to sync it. This changes what the sync skips, so update every Windows computer
  together: an older one keeps retrying these names.
- **Sync: a file moved onto one renamed elsewhere keeps both. Upgrade all devices together.** When you moved a
  file onto another one (replacing it) while that one was renamed on another computer or in the browser, the sync
  could take one file as two moves: the next pass then wrote over the renamed file in the cloud (its old bytes kept
  only as a version), or the two sides kept different files under one name. The same the other way round (a file
  moved onto another in the cloud while that one was renamed here) could replace the renamed file here. Now the
  moved file goes to its new place and the renamed one is kept under its new name, on both sides. This changes how
  the sync decides, so update every computer together: an older one can still make the old plan.
- **Sync: a cloud file listed without a version is never trashed. Upgrade all devices together.** Before a file
  you deleted on your computer goes to the cloud trash, the sync checks that the cloud file is still the version it
  last saw. A file the cloud listed without a version (an ETag) passed that check, so it could go to the trash
  although it could not be shown unchanged; a whole folder holding such a file could too. Now such a file stays in
  the cloud and comes back down, and the activity list says so once; a file renamed or moved on your computer is
  uploaded under its new name instead of being moved in the cloud. This changes how the sync decides, so update
  every computer together: an older one can still trash such a file.
- **Server: sign-in cookies follow the public address.** When `CRAFTCLOUD_PUBLIC_URL` starts with `https://`, the
  sign-in cookies are marked Secure on their own; `OXICLOUD_COOKIE_SECURE` is now needed only to override that.
  `.env.example` no longer sets it to `false`: if your `.env` was copied from an older one and you serve CraftCloud
  over https, delete the `OXICLOUD_COOKIE_SECURE=false` line (or set it to `true`).
- **Server: https stays https.** When `CRAFTCLOUD_PUBLIC_URL` starts with `https://`, every answer carries
  `Strict-Transport-Security: max-age=15552000` (180 days, this host only, no preload), so browsers keep using https
  for your CraftCloud. On plain http nothing changes. If your reverse proxy already sends this header, browsers see
  it twice with the same meaning; you can drop it there.
- **Server: stricter settings writes.** A change to the server settings (Admin › Server) is refused with 413 above
  4 KiB, and with 400 when it carries a field other than `web_apps` and `disabled_apps`, instead of ignoring it.
  The web frontend sends only those two, so nothing changes for people using it; a script that writes the settings
  must send just these fields.

### New

- **Web: share links from the file browser.** A file's or folder's "…" menu (or SHARE with one item selected) has
  "Share link…": an expiry (never, 1, 7 or 30 days, or a day picked in a calendar), an optional password, and the
  link to COPY. A link the server refuses says why in plain words.
- **Web: your share links in one place.** "Your share links" in the account menu lists every link you made: the
  file or folder (where it is now, or the name it was shared with if it's gone), its expiry and whether it has a
  password, with COPY and REVOKE.
- **Web: the public share page is reworked.** Subfolders open in place, a folder lists up to 1000 items (it used to
  show 200 and call that the whole count), long names fit on a phone, and a folder still loading or failing to load
  no longer looks empty.
- **Web: a file's version history.** "Versions…" in a file's "…" menu lists the current file and the 5 kept
  versions, each to download or restore. A restore keeps the current file as a version first and never overwrites a
  change made elsewhere in the meantime. "Delete" is now called "Move to trash".
- **Web: single sign-on is easier to set up.** Test connection says what it is doing and why a test failed, in plain
  words, and password sign-in can't be turned off until a test of the issuer in the form has worked. The "Sign in
  with …" button spans the sign-in sheet.
- **Desktop notifications on Linux** for a conflict, a sign-in that ran out and a sync that can't go on; clicking
  one opens the window. The tray no longer flickers between "Up to date" and "Checking…" on every pass (Hyprland,
  Sway and other bars). A device removed on the server now shows "Signed out" (sign in again) instead of the error
  "Session not found".
- **Desktop: one CraftCloud at a time, on every system.** Starting CraftCloud again while it runs (from the Start
  menu, the Dock or the app menu) brings the running one's window to the front, also when it is minimised, instead of
  opening a second copy that said "another CraftCloud is already syncing". A start at login while one runs ends
  quietly.
- **Windows: the command line prints in a terminal** (#4). `craftcloud status` in Command Prompt or PowerShell
  showed nothing, because `craftcloud.exe` is a GUI program. The MSI and the portable zip now also hold
  `craftcloud.com`, a console program that runs `craftcloud.exe` and shows its output; both shells pick the `.com`
  when you type `craftcloud`. The MSI adds its folder to your PATH (for you only): open a new terminal after
  updating. Scripts that read the output through a pipe or a file work as before.
- **CLI: Windows asks which app to use.** `craftcloud open` (and a `craftcloud://open` link) on a file whose type
  has no app yet, such as `.md` or `.docx` on a fresh Windows, now shows the "Open with" dialog instead of doing
  nothing. `craftcloud open --json` then adds `"with": "chooser"` to `{"opened": …}`.
- **Nix: a Home Manager module** for the desktop client (`programs.craftcloud`: start at login, `craftcloud://`
  links), and the NixOS server module sets up its database in the right order on NixOS 25.11 and later.

### Fixes

- **Desktop: Show brings back a minimised window** (#2). With the window minimised to the taskbar, the tray's
  "Show CraftCloud" (and starting CraftCloud again) left it there; a window closed to the tray came back only after a
  few seconds. The tray, Quit and the status line now keep working while the window is minimised or hidden, and
  Show brings the window out of the taskbar and in front at once.
- **Desktop: the window comes back at its size.** A window minimised or hidden in the tray when CraftCloud quit
  (or when it saved its place, every 30 seconds) came back as a 64×64 square in the screen's corner on Windows. The
  window's place is now saved only while it is shown, and a window that would come back smaller than it can be
  opens at its default size in the middle of the screen, once; after that it keeps its place again.
- **Desktop: a sign-in on the command line reaches the window.** After `craftcloud login <server>`, the desktop
  client stayed on its first setup step with the server filled in, even after a restart. It now starts signed in
  with that sign-in and goes on to choosing folders (or straight to syncing when a folder is set); a client already
  running and waiting for a sign-in takes it at once.
- **Windows: uninstalling turns off start at login.** The MSI used to leave the "Start at login" entry behind,
  starting a program that was no longer there. Uninstalling now removes it (and takes the folder off PATH); an
  update keeps it. If you uninstalled an earlier version with start at login on, remove the leftover entry with
  `reg delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v CraftCloud /f` (or turn it off in Task Manager › Startup apps).
- **Sync: a failed download leaves no part file.** A download that couldn't be put in place used to leave its
  hidden `.<name>.<version>.craftcloud-part` file in the sync folder. It is removed now on every failure but a
  broken-off transfer (kept to resume from), and part files left for more than a day (after a crash) are cleaned up.
- **Sync: a folder replaced by a file syncs in one pass.** When a synced folder was replaced by a file of the same
  name (or a file by a folder) and files had been moved out of it, the sync needed extra passes, with "couldn't
  move" errors in the activity list on the way; nothing was lost. The steps now run in an order that works the
  first time. A file moved to where its own folder was (`Photos/Photos` renamed to `Photos`, with the folder gone)
  used to fail on every pass; it now syncs as a new file in its new place (the old one in the cloud goes to its
  trash, as a delete does, so a share link to it stops working and its kept versions stay under the old name).
  Nothing to do when updating: devices on different versions still agree on every file.
- **Sign-out: signing out now also ends the session on the server.** `craftcloud logout` and the desktop client's
  Sign out forgot the sign-in on the computer, but the server kept the session open (it needs the device's refresh
  token to know which one to end, and the client didn't send it). Now both end it there too. When the server
  can't be told (offline, or it refuses), the sign-in is still forgotten on the computer and they say so:
  `craftcloud logout` then exits with code 8 or 1 and `--json` has `"server_session_ended": false`; the desktop
  client shows a note on the sign-in screen.
- **Desktop: the tray says Resume only when paused** (#25). Signed out, or waiting for you before a mass delete, the
  tray menu offered "Resume syncing", and picking it paused syncing. It now says "Pause syncing" there, as it does.
- **Logging: trace logs no longer include HTTP request bodies and headers.** With `RUST_LOG=trace`, the desktop client,
  the command line and `craftcloud-admin` logged the raw bytes of every request and answer. The HTTP library's
  lines now stop at debug level (method, address, status), whatever `RUST_LOG` asks for.
- **Versions: a kept version's time is never after it was replaced.** `craftcloud versions` (and `--json`'s `modified`)
  and the web frontend's Versions sheet show when a version's content was last changed. That time comes from the
  server and the replacement time from the computer that replaced it, so a server that rounds up or whose clock
  runs ahead could show a version as changed a second (or more) after it was replaced. Then the replacement time is
  shown instead. Versions already kept are shown this way too; nothing is renamed.
- **Desktop: long errors stay in their row** in the activity list (the full text on hover), and "Syncing stopped"
  no longer ends with two full stops.
- **Windows: a start at login stays in the tray** (#9). `craftcloud.exe --hidden`, the command that Start at
  login runs, showed the window a moment after it started. The window library shows every window itself once it
  has drawn its first frame, whatever was asked. Now a window that isn't meant to be seen hides itself again in the
  same step, so a start at login shows nothing; the tray's Show CraftCloud brings the window up as before.
- **Linux on X11: a start at login stays in the tray too.** In an X11 session (Xfce, GNOME on Xorg, KDE on X11)
  `craftcloud --hidden` opened the window for the same reason as on Windows, and it now stays in the tray the same
  way. On every system a hidden start keeps the window hidden only where there is a tray to come back from:
  without a tray host (stock GNOME without the AppIndicator extension) or a tray icon, the window opens, since it
  is then the only way to the client. A tray host that comes up more than three seconds after the client is too
  late for that start.
- **Windows: a window hidden in the tray uses next to no CPU.** With the window closed into the tray, CraftCloud
  kept drawing a frame of it every second (on Windows the window library can't tell a hidden window from a shown
  one), which on a computer drawing in software kept a core busy. A hidden window now draws nothing; the frame
  that shows it comes with Show.
- **`craftcloud status` says which computer can't have a name.** A file left out because Windows can't have its
  name (`a:b.txt`, `CON.txt`, a trailing dot) was listed as "a name the cloud can't take". It now says "a name this
  computer (Windows) can't have", and two cloud names that differ only in upper and lower case say so. `--json`'s
  `reason` stays `name`.
- **Linux: without a tray host, closing the window quits.** On a desktop with no StatusNotifier host (stock
  GNOME without the AppIndicator extension) the client took its icon as shown when nothing showed it, so
  closing the window left it running with no icon and no window, and a start at login showed nothing at all.
  It now follows the host on the session bus: with none, closing the window quits and `--hidden` opens the
  window; when a host comes up later (a bar that starts after the client), the icon shows and the window hides
  into it as before. On Sway or Hyprland a bar that starts more than three seconds after `craftcloud --hidden`
  now gets a window at login; start the bar first, or the client after it.
- **Desktop: the pause switch says what it does.** Settings › On this computer had "Pause syncing" with Off and On,
  so "Off" meant syncing. It is now **Syncing**, with **Paused** and **Active**. The tray menu keeps "Pause syncing"
  and "Resume syncing".
- **Desktop: an idle window uses next to no CPU** (#3). The desktop client's window drew itself four times a second for as
  long as it was open, even with nothing to show. Without a graphics card (a virtual machine, or a computer that
  draws in software) that kept several cores busy. Now it draws when something changes (a sync, an answer from
  the server, the mouse or keyboard) and otherwise once a minute, to keep "Synced 2 min ago" true.
- **CLI: a server that can't be reached gives exit code 8 on Windows too.** The command line read the
  system's error message to tell an unreachable server from other errors, and Windows words it differently, so
  there such a command exited with 1. It now goes by the kind of error, the same on every system and in every
  language. A server name that can't be looked up now also gives 8 on Linux, as do a connection that was reset
  and one that timed out.
- **Desktop: an offline window no longer asks the server again and again.** The main window asks the server who is
  signed in (for Settings). When that failed (the computer offline, the server down or answering with an error), it
  asked again on the very next frame and redrew for every answer, without pause, for as long as the window was open.
  Now it waits 5 seconds after the first failure, twice as long after each one that follows, up to 2 minutes, and
  starts afresh once an answer comes or the sign-in changes.

### For people building CraftCloud

- **Rust 1.100.0 is pinned** (`rust-toolchain.toml`) for local builds, CI, the release builds and the image. The
  oldest Rust that builds CraftCloud stays 1.95.
- **The image build no longer needs Python:** the Craft web builds are fetched by `cargo xtask fetch-apps` and their
  pins moved by `cargo xtask bump-apps`.
- **`cargo xtask e2e-sync`** runs a two-client sync test against any server ([TESTING.md](TESTING.md)), and a
  mixed pair too (`--bin-laptop`, `--bin-studio`: an older release on one side, this tree on the other), with cases
  for conflicts, a deleted folder, a restored version, names Windows can't have and leftovers.
- **`cargo xtask e2e-web`** runs the web frontend in headless Chromium against a fresh server: setup, sign-out and
  sign-in, an upload, a share link opened signed out, a device approval and the admin CSRF check.
- **More tests in `cargo xtask ci`:** property tests of the sync decisions (`crates/core/src/reconcile/properties.rs`,
  proptest) check the rules in AGENTS.md on generated states and listings; the sync engine, the checks before a
  trash and the sign-in refresh run against a scripted local server (`crates/native/src/fake_server.rs`).
- **The live sync tests run in CI** on every pull request, against a server image built from the tree; they keep to
  folders of their own and clean up after themselves, so they also pass on an account with leftovers. The Release
  workflow runs the workspace tests on Windows x64 too, and the draft release waits for them.
- **Coverage:** `cargo llvm-cov` by hand, with the commands and the gaps worth a test in
  [TESTING.md › Coverage](TESTING.md#coverage).
- **The Craft web app pins are checked before a release:** the release checklist ([RELEASING.md](RELEASING.md)) now starts with
  `cargo xtask bump-apps --check`, and the Release workflow reports the Craft web app pins in its run summary.

## 0.1.1

Fixes for 0.1.0. **Update the server and every desktop client.** The sync fix (a file being saved is never
overwritten) protects only the devices that run 0.1.1, so update them all. Nothing in your settings or data needs
changing.

- **`craftcloud open` hands over and returns.** The app it opens no longer inherits the command's output, so a
  script that reads that output (`path=$(craftcloud open …)`) gets it at once instead of waiting until the app
  quits (#6). On Windows it prints and opens the usual `C:\…` form of the path instead of `\\?\C:\…` (#5).
- **Server: files opened in the browser stay files.** HTML, SVG and XML that people store are served with a
  sandbox policy, so they never run as pages on your CraftCloud's address. WebDAV names with a backslash are refused
  (as the desktop client already does).
- **Sync: a file you're saving is never overwritten.** A file changed in the last two seconds waits for the next
  pass instead of looking unchanged, so a change made in the cloud at the same time now ends in a conflict copy
  rather than replacing your edit (#36). A download also checks the local file one last time before replacing it.
- **`craftcloud sync --once` no longer says "up to date" while a file waits.** A file written in the two seconds
  before it ran was held for the next pass, yet the result said `up_to_date` (#15). Now `--once` waits those two
  seconds once and syncs the file in a second pass; anything still being written after that is listed in the new
  `waiting` field (also in `status --json`) and in the headline ("1 file is still being saved"). The exit code
  stays 0: the file waits for time, not for you.
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
