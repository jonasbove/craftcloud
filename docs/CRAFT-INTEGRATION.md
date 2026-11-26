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
integrator meets are 2 (a bad `--app` or path), 3 (not signed in), 4 (not found), 5 (already there: pass `--force`;
with `save --if-match`, changed elsewhere),
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

`--app` everywhere below takes an `id` from this list, in any case. A desktop app without a web build
(`web = false` in `apps.toml`; none today) is listed and taken by `--app` like the others; only the `-apps` image
leaves it out. An id is stored, so it stays when an app is renamed: LoveCraft is `prosecraft-rs`, and opens
`.prosecraft` backups.

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
- The server listing also carries the server's `id` and, for each file, its version tag `etag` in its HTTP form
  (`"\"9ff164e6f5312e5a-1794606820\""`), the one `save --if-match` takes (section 3.4); the client's listing has
  neither.

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

### 3.4 Save to CraftCloud: `craftcloud save <file> [--to FOLDER] [--wait] [--force | --if-match ETAG] --json`

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
  Uploaded directly, a save without `--force` is written with `If-None-Match: *`: it creates the file and never
  replaces one, even one that appears meanwhile (exit 5).
- **Only over the version you read:** `--if-match ETAG` replaces the cloud file only while the server still has
  that version (`ETAG` from `files`/`ls --json`). It always goes straight to the server, even with a client
  running, makes no folder, keeps the old version first, and writes with `If-Match`. When the file changed, moved
  or went elsewhere meanwhile, nothing is written and it exits 5 (`conflict`): reload, or save under another name.
  It can't be combined with `--force`.

```json
{"path": "WordCraft/notes.md", "local_path": "/home/john/CraftCloud/WordCraft/notes.md",
 "via": "client", "state": "synced", "replaced": false, "size": 812, "etag": null}
```

`via` is `client` (`state` `queued`, or `synced` with `--wait`; `etag` `null`) or `server` (`state` `uploaded`,
`local_path` `null`, `etag` the new version's tag, for the next `--if-match`). After a save through the client, the app can keep working on `local_path`: every later save there is
synced like any other file.

### 3.5 A cloud file without the sync folder: read, then save over what you read

An app that works on a cloud file without the sync folder (a cloud backend that runs `craftcloud … --json`) uses
three calls:

1. **The version:** `craftcloud ls "Documents/notes.md" --json` (or `files --in FOLDER --json` without a client
   running) gives the file's `etag`.
2. **The bytes:** `craftcloud get "Documents/notes.md" -` writes them to stdout and nothing else (errors to stderr,
   as JSON with `--json`; a non-zero exit means the bytes are not the whole file). Take the `etag` *before* reading:
   the other order can pair older bytes with a newer tag and let the guard pass over a change.
3. **Saving back:** `craftcloud save notes.md --to Documents --if-match ETAG --json` writes only over that version
   and answers with the new `etag` for the next save; exit 5 means it changed elsewhere and nothing was written. A
   new file is `save` without `--force` or `--if-match`, which never replaces one.

### 3.6 The control channel underneath

`files` talks to the running client over its control channel: one JSON line to a loopback port, with the secret
from `control.json` in the client's settings folder (readable only by the user; [CLI.md](CLI.md), "The desktop
client's sync"). The request is `{"secret": "…", "request": {"cmd": "files", "extensions": ["docx", "md"]}}` and the
answer `{"files": [{"path", "name", "local_path", "size", "modified", "sync"}]}`, or `{"error": "…"}`
when the client isn't signed in. Integrators should call the `craftcloud` command rather than the channel: the
command is the stable interface and handles the fallbacks; the channel may change with the client.

**Tests.** The extension filter and the built-in table (checked against `apps.toml`) in `crates/core/src/catalog.rs`;
the local view (excluded folders, `.craftcloud`, temporary files, sync states) in `crates/native/src/local_files.rs`;
the `files` request and its fallbacks in `control.rs`; `--app` filtering, the JSON shapes, where `save` puts a file
and its overwrite rules in `apps/craftcloud/src/cli/craft.rs` and `output.rs`; what a write may replace (`--force`,
`--if-match`) in `files.rs`; `ls`/`save --if-match`/`get -` against a real server in `apps/craftcloud/tests/cli_live.rs`.

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

The apps share libraries (`craft-libs`). Their Open, Save and Share pages already exist there, vendor-neutral:
`craft-cloud` holds the contract (an `Entry`, a `Capability`, a `Permission`) and `craft-cloud-ui` the pages. The
pages never reach a cloud themselves. They hand a `Job` to a `CloudBackend` (`fn submit(&self, job: Job, post:
Post)`) and draw the `Event`s it sends back; a failure is a `Fault`, and `Fault::Conflict` is a conditional write
the server refused. The share dialog has its own `ShareBackend`. `craft-cloud-local` (a folder on disk) is the
reference backend.

No backend for a WebDAV server exists yet: that is step 2 of RFC-0024 in `storytold/craft-contrib`, and craft-libs#2179
notes it has not started. craft-libs#2452 (Google Drive as a `CloudBackend` of its own) is the precedent for one
provider's backend. Nine app pull requests ("Open, Save and Share against cloud storage") wait for a backend to
run against.

**The proposal: a `CloudBackend` in craft-libs that runs `craftcloud`.** It lives in craft-libs, next to the
others, and does what section 3 does: it runs the `craftcloud` program with `--json` and reads its answer. It speaks
no HTTP and holds no token. So the sign-in, the kept versions and the write guards stay in CraftCloud, as section 3
promises. Without `craftcloud` on the computer, the backend reports no account and the pages offer nothing.

| `Job` | `craftcloud` command | Notes |
|---|---|---|
| `Account` | `status --json` (and `server info --json`) | Who is signed in and where. Not signed in (exit 3) is `Fault::NotLinked`; the capabilities are conditional writes and links |
| `SignIn` | `login --no-browser --json` | The client's browser sign-in: the first line's `verification_uri` becomes `Event::Browser`, the second line the account ([CLI.md › JSON output](CLI.md#json-output)) |
| `LinkStart`, `LinkPoll` | the same sign-in | CraftCloud's storage is the account itself, so there is nothing separate to link: signed in is linked |
| `List` | `files --json` (`--in FOLDER`) | Each file with the server's current `etag`, so a later write can be guarded (§ 3.2) |
| `Read` | `get PATH -`: the file's bytes on stdout | The `etag` comes from the listing read just before; a write over it is refused if the file changed in between (§ 3.5) |
| `Write` | `save --if-match <etag>` | Refused when the file changed, moved or went since that `etag` (exit 5, JSON error kind `conflict`): `Fault::Conflict`. The old bytes are kept as a version first, as for every overwrite |
| `Create` | `save` without `--force` | Never replaces: a name already taken is refused (exit 5), and the page asks for another |
| `SignOut` | `logout --json` | This signs the whole computer out of CraftCloud, the desktop client too, so the page should say so |

**What CraftCloud provides for this** (§ 3.2, § 3.4 and § 3.5): an `etag` on each file in `craftcloud files --json`
and the other listings, `craftcloud save --if-match <etag>`, which writes only over that version and otherwise
answers exit 5 with the JSON error kind `conflict`, and `craftcloud get PATH -`, which writes only the file's bytes
to stdout.

**Left unsupported:** `Conversation` (CraftCloud has no calls or chat rooms), contacts and sharing with a person
(OxiCloud's user shares aren't in the CLI), and notifications. The backend reports none of these capabilities, so
the pages hide them rather than offer something that fails. `SharedWithMe` answers an empty list.

**Gaps:**

- **No rename-stable id.** An `Entry` has a `room` that must survive a rename. WebDAV has no file id (section 3,
  and the kept versions are keyed by path for the same reason), so the backend can only use the path, and a room
  splits when the file is renamed or moved. The server listing's `id` (OxiCloud's REST id) is stable, but other
  servers have none.
- **Links carry no permission.** The share dialog offers view or edit (`Permission`). A CraftCloud share link
  (`craftcloud share create`) has an expiry and an optional password but no view or edit setting, so the backend
  makes links only and doesn't offer the choice.

Until the backend exists, section 1 gives the same result with the system dialogs, and an app can call section 3
itself, as LoveCraft does.

### 5.1 Suite accounts

craft-libs also has `craft-suite-account`: one account list for the whole suite (`accounts.json` in the suite's
settings folder, `<config>/artcraft/`), with one token per account in the OS keychain (service
`ai.storyteller.artcraft.suite-account`). CraftCloud doesn't write its sign-in there for now, and keeps it in its
own keychain entry. The reasons:

- Nothing reads the store yet. The only consumer in sight is an open craft-launcher pull request (#366), for a
  CollabCraft session.
- An account row has no kind or provider field. A reader can't tell a CraftCloud row from another kind of
  deployment, and would treat it as one.
- `sign_in` always makes the new account the active one, so storing CraftCloud's sign-in would change which account
  the other apps use.
- OxiCloud rotates refresh tokens and ends a sign-in when a spent one is used again. The client refreshes under its
  own lock file (`crates/native/src/session.rs`). An app refreshing the same token from the suite store would not
  take that lock, and the two could end each other's sign-in.
- A device token has the same rights as the account ([STATUS.md](STATUS.md)). Through `craftcloud` an app reaches
  the account only through the commands above; a token in the suite store would carry all of its rights to every
  app that reads it.

The asks that would change this are in [UPSTREAM.md § F2](UPSTREAM.md#f2-craft-suite-account-what-a-provider-like-craftcloud-would-need).
Until then the backend above is the way in, and it needs no token in the apps.

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

Apps with the bridge built in (WordCraft, GridCraft, DeckCraft, PdfCraft, FigCraft, CADCraft, FilmCraft, LoveCraft) need nothing
more. For the others the image injects a shim that speaks the same protocol on the app's behalf (marked experimental);
`apps.toml` records which app uses which.
