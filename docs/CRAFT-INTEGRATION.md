# CraftCloud and the Craft apps

How the Craft apps open files from CraftCloud and save them back, from what works today to what needs changes in
the Craft apps or the ArtCraft Launcher. The CraftCloud side of each step lives in this repository; anything that
needs a change upstream is a proposal, filed only after it has been agreed.

## 1. The sync folder (works today, every app)

The desktop client keeps a local folder (`~/CraftCloud` by default) in step with the server. Every Craft app already
opens and saves ordinary files, so every app works with CraftCloud without any change:

- **Open:** File › Open in the app, go to the CraftCloud folder. On Linux the system file dialog (xdg portal) lists it
  like any folder; adding it to the dialog's bookmarks makes it one click away.
- **Save:** save into the CraftCloud folder. The client uploads the file within a few seconds, keeps the previous
  version (the last 5) and makes a conflict copy if the file changed elsewhere in the meantime.
- **Which app opens a file** is the desktop's own choice (the default app for `.docx`, `.psd`, …). With the Craft apps
  installed through the ArtCraft Launcher, those are the Craft apps.

## 2. Deep links: `craftcloud://`

The desktop client registers the `craftcloud://` URL scheme:

| Link | What happens |
|---|---|
| `craftcloud://open?path=/Documents/Q3%20report.docx` | Opens the local synced copy with the default app (a Craft app, normally) |
| `craftcloud://reveal?path=/Documents/Q3%20report.docx` | Shows the file in the file manager |
| `craftcloud://open/Documents/Q3%20report.docx` | The same as `open?path=/Documents/Q3%20report.docx`: the path may also follow the action (and `reveal/…` likewise) |

The path is the file's path on the server. It must resolve inside the sync folder; anything else (`..`, absolute
paths elsewhere, a folder that isn't synced) is refused. If the client is already running, the link goes to the
running instance. Links let the web file browser, a chat message or another app say "open this in its Craft app".

**Built** (`crates/core/src/deeplink.rs`, `crates/native/src/open.rs`, `crates/native/src/url_scheme.rs`):

- **Reading a link.** The action is the host (`craftcloud://open?…`) or the path (`craftcloud:open?…`), any case.
  `path` is percent-encoded (`+` is a space; a literal `+` is `%2B`); a leading `/` is the cloud root. Without
  `?path=`, what follows the action is the path (`craftcloud://open/Documents/a.txt`; percent-decoded, `+` stays
  a `+`), checked the same way. No path at all says "this link has no path". Refused
  before anything touches the disk: another action, an empty path, `.` or `..` parts, backslashes,
  control characters, and CraftCloud's own `.craftcloud` folder.
- **Finding the file.** There must be a sync folder, and the path must be in a folder this computer syncs (not an
  excluded top-level folder; a top-level file only when top-level files sync). The local path is then resolved with
  symlinks followed and must still be inside the sync folder, so a symlink out of it is refused. A file that isn't
  downloaded yet says so ("isn't on this computer (yet)"); nothing crashes and nothing else is opened.
- **Acting.** `open` hands the file to the system's default app (`xdg-open`, `open`, `explorer`); `reveal` selects
  it in Finder or Explorer, and opens its folder on Linux (`xdg-open` can't select).
- **One instance.** The system starts `craftcloud <link>`. If a desktop client or `craftcloud daemon` runs, the
  link goes to it over the existing control channel (`control.json`, a `{"cmd":"link","url":…}` request with the
  secret); the running client checks and opens it, and the second process exits with its answer. Otherwise the
  second process does the same itself and exits; it never starts a window for a link. Exit codes: docs/CLI.md,
  "Links". With no terminal, a refusal shows as a desktop notification on Linux (`notify-send`).
- **Registration.**
  - Linux: `packaging/linux/io.github.jonasbove.craftcloud.desktop` (named for the app id, which is also the
    window's Wayland app id and X11 class) has `Exec=craftcloud %u` and `MimeType=x-scheme-handler/craftcloud;`
    (`desktop-file-validate` clean). The deb and rpm refresh the MIME cache (`update-desktop-database`); the
    tarball's `install.sh` also runs `xdg-mime default io.github.jonasbove.craftcloud.desktop
    x-scheme-handler/craftcloud`; the AppImage's `AppRun` installs its own entry with `Exec="<AppImage>" %u`.
  - Windows: the client writes `HKCU\Software\Classes\craftcloud` (`"<exe>" "%1"`) each time the desktop client or
    `craftcloud daemon` starts, not only with "Start at login": no admin rights are needed, and it keeps pointing
    at the current executable after an update moves it. The MSI writes the same values and removes them on
    uninstall.
  - macOS: `CFBundleURLTypes` in `packaging/macos/Info.plist.in` (bundle id `io.github.jonasbove.craftcloud`)
    registers the scheme. **Not working yet:** macOS
    hands the link to the app as an Apple Event (`kAEGetURL`), not on the command line, and the client has no
    handler for it (winit doesn't expose one). Until then a click starts or focuses CraftCloud without opening the
    file; `craftcloud 'craftcloud://…'` from a terminal works.
- **Tests.** Parsing (percent-encoding, `..`, absolute paths, `.craftcloud`, round trips) in `deeplink.rs`;
  excluded folders, not-yet-downloaded files and symlink escapes (Unix) in `open.rs`; the hand-off to a running
  client over the control channel, and the fallback when it isn't signed in or none runs, in `control.rs`.

## 3. The integration contract: the `craftcloud` command

The launcher and the apps never speak OxiCloud's API and never see the user's sign-in. They run the `craftcloud`
program (the desktop client's own command line, installed with it) with `--json` and read one JSON value from
stdout. Everything below is stable: fields may be added, existing ones keep their meaning. Errors go to stderr as
`{"error": {"kind", "message", "exit_code"}}` with the exit code ([CLI.md](CLI.md), "Exit codes"); the ones an
integrator meets are 2 (a bad `--app` or path), 3 (not signed in), 4 (not found), 5 (already there: pass `--force`),
6 (the client stopped mid-`--wait`) and 8 (server unreachable).

**Is CraftCloud there?** Look for `craftcloud` on `PATH` (Windows: `craftcloud.exe`) or for the `craftcloud://` URL
scheme. Without it, show nothing.

### 3.1 Which app opens what: `craftcloud apps --json`

Built into the program from `apps.toml`, so it works offline and without a sign-in. `opens` is every extension the
app can open; `default` the ones it claims when a file is opened by its type (each extension has exactly one
default app; `.md` and `.txt` are WordCraft's).

```json
[{"id": "wordcraft", "name": "WordCraft", "experimental": false,
  "opens": ["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "fodt", "rtf", "txt", "md", "html", "htm", "tex"],
  "default": ["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "fodt", "rtf", "txt", "md"]},
 {"id": "gridcraft", "name": "GridCraft", "experimental": false, "opens": ["xlsx", "…"], "default": ["xlsx", "…"]}]
```

`--app` everywhere below takes an `id` from this list, in any case. A desktop app without a web build yet
(`web = false` in `apps.toml`: ProseCraft-rs, id `prosecraft-rs`, which opens `.prosecraft` backups) is listed and
taken by `--app` like the others; only the `-apps` image leaves it out.

### 3.2 "In CraftCloud": `craftcloud files --app <id> --json`

The synced files on this computer that the app opens, sorted by path. With the desktop client (or `craftcloud
daemon`) running, the client answers over its local control channel from the sync folder and its own records:
no network, works offline, fast enough to call whenever a page opens. With none running (or one that isn't signed
in), the command lists the cloud instead, one folder at a time (slower; it needs the server). `--in FOLDER` limits it
to one cloud folder.

```json
[{"path": "Documents/Q3 report.docx", "name": "Q3 report.docx", "type": "file", "size": 18234,
  "modified": "2026-11-13T21:54:00Z", "starred": false,
  "local_path": "/home/john/CraftCloud/Documents/Q3 report.docx", "sync": "synced"},
 {"path": "WordCraft/notes.md", "name": "notes.md", "type": "file", "size": 812,
  "modified": "2026-11-14T08:02:11Z", "starred": false,
  "local_path": "/home/john/CraftCloud/WordCraft/notes.md", "sync": "pending"}]
```

- `path` is the cloud path; `local_path` the synced copy to open (or `null` when it isn't on this computer, which
  only happens in the server listing).
- `sync`: `synced`, `pending` (new or changed; the next sync moves it), `syncing`, `error`, `conflict` (these three
  only from a running client), `excluded` (a folder this computer doesn't sync).
- The server listing also carries the server's `id`; the client's doesn't.

**Recent, starred and search** are the server's, filtered the same way, with the same `local_path` and `sync`:

```sh
craftcloud recent --app wordcraft --json        # newest first; adds "opened_at"
craftcloud starred --app gridcraft --json       # adds "starred_at"
craftcloud search q3 --app deckcraft --json     # adds "score"
craftcloud ls Documents --app pdfcraft --json   # one folder
```

These need the server (and the sign-in the client already has). Folders are left out of every `--app` listing.

### 3.3 Opening a file

Open `local_path` directly, or hand the cloud path to CraftCloud, which checks that it is inside the sync folder
first: `craftcloud open "Documents/Q3 report.docx" --json` (prints `{"opened": local path}`), or the link
`craftcloud://open?path=/Documents/Q3%20report.docx` (section 2). A file whose `local_path` is `null` isn't here:
`open` exits 4 and says why.

### 3.4 Save to CraftCloud: `craftcloud save <file> [--to FOLDER] [--wait] [--force] --json`

Saves a local file (one the app just wrote, e.g. in a temporary folder) into CraftCloud:

- **Where:** into `--to FOLDER` (a cloud folder, made if missing), or else into a folder named after the app:
  `--app`, or the app that opens the file by default (`notes.md` → `WordCraft/notes.md`). A file no app claims needs
  `--to`.
- **How:** with a client running and the folder synced, the file is copied into the sync folder (whole, through a
  temporary name the sync ignores) and the client uploads it; `--wait` returns once it has (up to 30 minutes).
  Otherwise (no client, or a folder this computer doesn't sync) it is uploaded directly.
- **Overwrites:** an existing file is kept (exit 5) unless `--force`; a forced overwrite keeps the old version in
  `.craftcloud/versions/`, as every overwrite does. Through the client the check is on this computer: a file that is
  only in the cloud so far (not downloaded yet) isn't refused, and the sync keeps both versions as a conflict copy.

```json
{"path": "WordCraft/notes.md", "local_path": "/home/john/CraftCloud/WordCraft/notes.md",
 "via": "client", "state": "synced", "replaced": false, "size": 812}
```

`via` is `client` (`state` `queued`, or `synced` with `--wait`) or `server` (`state` `uploaded`, `local_path`
`null`). After a save through the client, the app can keep working on `local_path`: every later save there is
synced like any other file.

### 3.5 The control channel underneath

`files` talks to the running client over its control channel: one JSON line to a loopback port, with the secret
from `control.json` in the client's settings folder (readable only by the user; [CLI.md](CLI.md), "The desktop
client's sync"). The request is `{"secret": "…", "request": {"cmd": "files", "extensions": ["docx", "md"]}}` and the
answer `{"files": [{"path", "name", "local_path", "size", "modified", "sync"}]}`, or `{"error": "…"}`
when the client isn't signed in. Integrators should call the `craftcloud` command rather than the channel: the
command is the stable interface and handles the fallbacks; the channel may change with the client.

**Tests.** The extension filter and the built-in table (checked against `apps.toml`) in `crates/core/src/catalog.rs`;
the local view (excluded folders, `.craftcloud`, temporary files, sync states) in `crates/native/src/local_files.rs`;
the `files` request and its fallbacks in `control.rs`; `--app` filtering, the JSON shapes, where `save` puts a file
and its overwrite rules in `apps/craftcloud/src/cli/craft.rs` and `output.rs`.

## 4. The ArtCraft Launcher (proposal, upstream)

The launcher shows every Craft app as a card. With CraftCloud installed it could show, per app, the CraftCloud files
that app opens, using only section 3:

- **Is it there:** `craftcloud` on `PATH`, or the `craftcloud://` scheme registered. If not, nothing changes.
- **"In CraftCloud" on the app's page:** `craftcloud files --app <id> --json` (offline, from the running client),
  shown recent first by `modified`, with each file's `sync` state as a small badge. A "Recent" tab can use
  `craftcloud recent --app <id> --json` when online.
- **Open:** open `local_path` with the app the card launches (or `craftcloud://open?path=…`), so the launcher never
  needs the user's CraftCloud sign-in.
- **The app ids** are the apps' lower-case names (`wordcraft`, `gridcraft`, …), as in `apps.toml`, which takes its names and colours from the launcher's catalog; `craftcloud apps
  --json` lets the launcher check which extensions CraftCloud lists for each.

This needs a change in the launcher, so it's a proposal to its authors, not something CraftCloud ships.

## 5. "Open from / Save to CraftCloud" inside the apps (proposal, upstream)

The apps share libraries (`craft-libs`). A small `craft-cloud` crate there could add two menu items to every app,
built on section 3 alone (it runs `craftcloud` and parses its JSON; no HTTP, no tokens):

- **Open from CraftCloud…** lists `craftcloud files --app <self> --json` in the app's own dialog (filter by name,
  sort by `modified`), then opens the chosen `local_path`. Files that aren't here (`local_path` `null`) are shown
  greyed out.
- **Save to CraftCloud…** writes the document to a temporary file, runs `craftcloud save <tmp> --to <folder> --wait
  --json` (the folder picked from the folders in `files`, default the app's own), and shows "Saving to CraftCloud…"
  until it returns. Exit 5 asks "Replace?" and repeats with `--force`; the app then reopens `local_path`, so later
  saves go straight into the sync folder.

Without `craftcloud`, the items are hidden. Until then, section 1 gives the same result with the system dialogs.

## 6. The Craft web apps in the browser

**Status:** planned. The `-apps` image serves the web builds and switches them on and off per app; the steps below
(the page that frames an app, and the shim) are not built yet ([ROADMAP.md](ROADMAP.md)).

The `-apps` image serves the Craft web builds under `/apps/<app>/`, same-origin with CraftCloud's frontend. The web file
browser opens a file in its app through `craft-web-host`, the apps' host protocol (`?host=parent`):

1. The browser loads `/apps/<app>/?host=parent` in an iframe and waits for `ready`.
2. It fetches the file's bytes (WebDAV, the signed-in session) and posts `open` with the name and bytes.
3. On `save`, it snapshots the current server copy to `.craftcloud/versions/`, then writes with `If-Match` on the ETag
   it opened. A `412` means the file changed meanwhile: the save becomes a conflict copy next to it.
4. `dirty` drives the "unsaved changes" warning; `failed` shows the app's error.

Apps with the bridge built in (WordCraft, GridCraft, DeckCraft, PdfCraft, FigCraft, CADCraft, FilmCraft) need nothing
more. For the others the image injects a shim that speaks the same protocol on the app's behalf (marked experimental);
`apps.toml` records which app uses which.
