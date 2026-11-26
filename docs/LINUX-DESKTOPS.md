# The desktop client on the Linux desktops

How the desktop client behaves on KDE Plasma, GNOME and Xfce, checked on 2026-11-16 against `main` at `7442f4c`
with a release build of `craftcloud` (0.1.1) and a throwaway server from `scripts/smoke-test`. Everything ran
headlessly, in Arch Linux containers on one x86_64 computer (the client was built on the same glibc), with the
desktops' own pieces started by hand where a full login session won't start in a container. What needs a real
session is marked so, and so is what wasn't tested ("Not tested") or rests on unit tests only.

Run these checks again before a release that changes the tray, the notifications, the keychain, start at login or
the window, and when a new major version of one of these desktops is out; then update the date, the commit and
the versions above.

The columns are the desktop as it ran, not a full login: **Xfce** 4.20 (`xfce4-session` under Xvfb, X11);
**GNOME** 51.1 (`gnome-shell --headless --wayland`, mutter's virtual monitor, Wayland); **KDE Plasma** 6.8.3
(`kwin_wayland --virtual --xwayland`, `kded6`, `plasmashell`, Wayland with Xwayland). The full list of
versions and how each session was started is at the end.

## The matrix

| Check | Xfce (X11) | GNOME (Wayland) | KDE Plasma (Wayland) |
|---|---|---|---|
| **Tray: registration** | Pass. `xfce4-panel`'s Status Tray plugin owns `org.kde.StatusNotifierWatcher`; `RegisteredStatusNotifierItems` lists `org.kde.StatusNotifierItem-<pid>-1`; the icon is in the panel with the sync dot (screenshot) | Stock GNOME: **no watcher** on the bus; the client registers its item anyway and logs `tray icon ready` (see "Bugs found"). With the AppIndicator extension (`gnome-shell-extension-appindicator` 66) switched on while the client ran, the watcher appeared, the item registered and the icon showed in the top bar with the sync dot (screenshot) | Pass. `kded6`'s StatusNotifierWatcher module owns the name; the item registers (`RegisteredStatusNotifierItems`). The panel itself was not captured (see "Screenshots on KDE") |
| **Tray: left click shows the window** | Pass (xdotool click on the icon: the hidden window mapped again) | Not tested headlessly: no pointer in the headless shell | Not tested headlessly: no pointer, no capture |
| **Tray: the menu** | Pass. Right click: status line, Open CraftCloud folder, Open in browser, Sync now, Pause syncing, Recent files, Show, Settings…, Quit; the items grey out with the state (folder missing); the tooltip shows the status line (screenshot) | Pass, opened through the shell (`org.gnome.Shell.Eval`, not a click): 11 entries, the same list (screenshot) | Not captured |
| **Closing the window** | Pass: the window hides, the client keeps syncing in the tray | **Stock GNOME: the client keeps running with no window and no icon** (bug, below). With the extension: closes into the tray (Wayland can't hide) | Not tested: closing needs a click or KWin scripting; the Wayland path is the same as GNOME's with a host |
| **`--hidden` (a start at login)** | **The window opens, visible** (bug, below: eframe shows every window after its first frame; applies to any X11 session) | Stock GNOME: **no window and no icon**, the client runs invisibly (bug, below). With the extension: no window, the icon in the top bar | Not tested headlessly |
| **Notifications: the service** | `org.freedesktop.Notifications` = Xfce Notify Daemon 0.9.7 | `org.freedesktop.Notifications` = gnome-shell 51.1 | `org.freedesktop.Notifications` = Plasma 6.8.3 (plasmashell) |
| **Notifications: one arrives** | Pass. The sync folder moved away → "Your CraftCloud folder is missing" with the CraftCloud icon and an "Open CraftCloud" button, 3 s after the move (dbus-monitor saw the `Notify`; screenshot) | Pass. The same notification as a banner with the icon (screenshot) | The `Notify` call reached Plasma's service 3 s after the move (dbus-monitor); Plasma's reply and the banner were not captured |
| **Notifications: clicking one** | Pass. A click on "Open CraftCloud" (`ActionInvoked`) brought the hidden window back | Not tested headlessly (no pointer) | Not tested headlessly |
| **Keychain** | **None** (no Secret Service: `org.freedesktop.secrets` has no owner and nothing activates it). The client logs `no keychain (… org.freedesktop.secrets was not provided by any .service files); keeping the tokens in a private file` and writes `~/.config/craftcloud/tokens.json`, mode 0600. Sign-in, sync and a restart worked | **GNOME Keyring** 51.1 (`gnome-keyring-daemon --unlock --components=secrets`). The tokens went into the keyring: `secret-tool search service CraftCloud` lists one item, label `<server>@CraftCloud:default`; no `tokens.json`; no warning | **KWallet's Secret Service** (`ksecretd`, kwallet 6.32, `kwalletrc` `[org.freedesktop.secrets] apiEnabled=true`): it owns `org.freedesktop.secrets` but has no wallet yet and lists no collections; the client logs `no keychain (Couldn't access platform secure storage: SS error: result not returned from SS API); keeping the tokens in a private file` and falls back to `tokens.json` (0600). Making the wallet needs KWallet's dialog: not testable headlessly. The client kept working either way |
| **Keychain: the window while it waits** | The window opened at once in every case; "Opening the system keychain…" was never on screen long enough to capture | same | same |
| **`craftcloud://` handler** | Pass. After `desktop-file-install` and `update-desktop-database`, `xdg-mime query default x-scheme-handler/craftcloud` → `io.github.jonasbove.craftcloud.desktop` | Pass (same) | Pass (same) |
| **`xdg-open` of a link / a synced file** | Pass. `craftcloud open Notes/notes.txt` and `xdg-open 'craftcloud://open?path=Notes/notes.txt'` (through exo-open) both reached the text/plain handler with the local path; `craftcloud://reveal` opened Thunar on the folder (screenshot) | Pass (through `gio open`); reveal opened Nautilus on the folder (screenshot) | Pass (through `kde-open`); the file manager wasn't captured |
| **Start at login: the entry** | Pass. `~/.config/autostart/io.github.jonasbove.craftcloud.desktop` written as `crates/native/src/login.rs` writes it (`Exec="/usr/local/bin/craftcloud" --hidden`, `X-GNOME-Autostart-enabled=true`); `xfce4-session` ran it at the session's start (`craftcloud --hidden` was running, the icon in the panel) | The entry's content is the same (unit tests in `login.rs`). Whether GNOME runs it: not testable headlessly: `gnome-session` 51 starts the session as systemd user units (`gnome-session@gnome.target`), which a container has no manager for | Not tried headlessly (`startplasma-wayland` wasn't run; KWin and plasmashell were started by hand) |
| **Start at login: the switch** | Not tested headlessly: turning **Settings › Start at login** on needs a click in the window; the entry was written by hand with the same content | same | same |
| **The window opens and draws** | Pass: 960×640, drawn in software (`WGPU_BACKEND=gl`, llvmpipe), screenshot of the X11 window under xfwm4 | Pass: a Wayland window, drawn, screenshot through `org.gnome.Shell.Screenshot`; the dock shows the CraftCloud icon for it (the app id matches the desktop entry) | Wayland-native: the client started, registered its tray item and answered `craftcloud status`; its window was captured only as an **Xwayland** window (`WINIT_UNIX_BACKEND=x11`, `import -window`), 960×640, drawn |
| **A second start shows the running window** | Pass (used throughout: starting `craftcloud` again showed the window, no second copy) | Not tested | Not tested |

What was not checked anywhere: a real login (the lock screen, the autostart firing at a real login, GNOME on Xorg,
KDE on X11), a locked keychain prompting at the start, the tray on Cinnamon, MATE or a bar, and a graphics
card (everything drew in software).

## Bugs found

- **A start at login on X11 opens the window.** `craftcloud --hidden` builds the window with `with_visible(false)`,
  but eframe shows every window after its first frame (`post_rendering` calls `set_visible(true)`, before the
  frame's viewport commands are handled), so on Xfce the window came up visible 1 s after a hidden start, both
  from `xfce4-session`'s autostart and by hand. The cause is in eframe, so it applies to any X11 session (GNOME
  on Xorg, KDE on X11); on Wayland a hidden start makes no window at all and is unaffected. Fixed in #90
  (the window's first pass hides it again where there is a tray), which builds on #86, below.
- **Without a tray host the client thinks the tray is up.** The StatusNotifierItem backend registers the icon
  whether or not a watcher is there (so a bar that starts later can show it), and the client took the
  registration as "ready": on stock GNOME closing the window left the client running with no icon and no window,
  and `--hidden` showed nothing at all. Starting it again from the launcher brings the window back (the control
  channel), which is the only way back. Fixed in #86 (the tray counts as up only while a
  host owns `org.kde.StatusNotifierWatcher`; without one, closing the window quits and `--hidden` opens it).
- **KWallet's Secret Service without a wallet** isn't usable by the client until a person has made the wallet in
  KWallet's dialog; the client falls back to the private file and says so in its log, and nothing in the window
  says where the sign-in is kept. Not a crash, and the fallback is the designed one; a line in Settings would help.

## Screenshots on KDE

KWin's own screenshot path ended its virtual backend both times it was tried (`spectacle -b -n -f`, which uses
`org.kde.KWin.ScreenShot2`, with the OpenGL and the QPainter compositor), and `grim` reports that the compositor
doesn't offer a capture protocol it knows. So on KDE the window was captured through Xwayland with X tools, and
the panel, its icon, the menu and the banner were not captured. The D-Bus facts (the watcher, the registration,
the notification service, the `Notify` call) stand on their own.

## How each session ran

One Arch Linux container per desktop (`archlinux:base`, `pacman -Syu` on 2026-11-16, host networking so the
client reached the server on the loopback), the client at `/usr/local/bin/craftcloud` with its desktop entry
and icons installed by `desktop-file-install`, a user `john`, a session bus at `$XDG_RUNTIME_DIR/bus`
(`dbus-daemon --session`), the client signed in through the device flow without a browser (the code approved
over the API, as `packaging/linux/test-package.sh` does), `RUST_LOG=info`, `WGPU_BACKEND=gl
LIBGL_ALWAYS_SOFTWARE=1`, and `dbus-monitor` on `org.freedesktop.Notifications` throughout. The notification
in every column was "folder missing": the sync folder moved away while the client ran, then moved back.

- **Xfce 4.20** (xfce4-session 4.20.4, xfce4-panel 4.20.8, xfwm4 4.20.0, xfce4-notifyd 0.9.7, exo 4.20.0, Thunar
  4.20.10): `Xvfb :77 -screen 0 1280x800x24`, then `xfce4-session`, which started xfwm4, xfce4-panel (the
  default layout, so no first-run dialog), xfdesktop, xfsettingsd, xfce4-notifyd and the autostart folder.
  `XDG_CURRENT_DESKTOP=XFCE`. The Secret Service activation file (GNOME Keyring's) was removed so that nothing
  could answer `org.freedesktop.secrets`. Screenshots with `import -window root`; clicks with `xdotool`.
- **GNOME 51.1** (gnome-shell 51.1, mutter 51.1, gnome-keyring 51.1, nautilus 51.1): `gnome-shell --headless
  --wayland --virtual-monitor 1280x800 --unsafe-mode` (unsafe mode only so `org.gnome.Shell.Eval` could close
  the window and open the indicator's menu). It needs a system bus and a machine id, and the container's
  `/run/systemd/{seats,sessions,users,shutdown}` removed, or it looks for logind and stops. GNOME Keyring unlocked
  with an empty password (`gnome-keyring-daemon --unlock --components=secrets`). `XDG_CURRENT_DESKTOP=GNOME`.
  Screenshots through `org.gnome.Shell.Screenshot`. `gnome-session --session=gnome` exits at once without
  systemd's user manager.
- **KDE Plasma 6.8.3** (kwin 6.8.3, plasma-workspace 6.8.3, plasma-desktop 6.8.3, kded 6.32, kwallet 6.32,
  kde-cli-tools 6.8.3): `kwin_wayland --virtual --xwayland --no-lockscreen --no-global-shortcuts --socket
  wayland-kde` (its `cap_sys_nice` file capability removed first, or it can't start in a container), `kded6`,
  `kactivitymanagerd` (by hand: the session bus didn't activate it, and plasmashell waits for it),
  `plasmashell --no-respawn`, `ksecretd`. `XDG_CURRENT_DESKTOP=KDE`, `KDE_FULL_SESSION=true`. The KDE
  processes print nothing without `QT_FORCE_STDERR_LOGGING=1`. The Secret Service activation file (GNOME
  Keyring's) was removed so that KWallet's was the one there.

Related: [TESTING.md](TESTING.md) for the other test layers, and the README's Linux notes for what each desktop
needs from the person.
