# CraftCloud: the command line

The desktop client is also a command line. With no command, `craftcloud` starts the desktop client as always
(`craftcloud --hidden` starts it in the tray). With a command, it runs that command and exits; it never opens a
window.

```sh
craftcloud login cloud.example.org      # sign in through the browser
craftcloud ls Documents                 # list a folder
craftcloud put report.pdf Documents/    # upload into a folder
craftcloud get Documents/report.pdf     # download into the current folder
craftcloud share create Documents/report.pdf --expires 7d
craftcloud status --json                # the desktop client's sync state, for scripts
```

`craftcloud --help` and `craftcloud <command> --help` list every option, with examples: every command has a
one-line summary and at least one example (a test checks it).

## Basics

- **Paths** are cloud paths, from the top of your cloud: `Documents/report.pdf`. Leading and trailing slashes
  don't matter, except that a trailing slash on a destination means "into this folder". `.` and `..` are refused.
- **The server** is the desktop client's, unless `--server URL` (or `CRAFTCLOUD_SERVER`) names another.
- **The sign-in** is the desktop client's: `craftcloud login` uses the same browser sign-in (OAuth device flow)
  and keeps the tokens in the same place (the system keychain). Signing in or out on one signs in or out on both.
  The client and the command line can run at the same time: a token refresh is done under a lock, so neither
  ever replays a token the other already used (OxiCloud ends the whole sign-in when that happens).
- **Passwords** (for share links and people) are never command-line arguments. They are read from the terminal
  without echo, or, when stdin isn't a terminal, from its first line: `printf '%s\n' "$PW" | craftcloud share
  create a.pdf --password`.
- **CraftCloud's own folder** (`.craftcloud`, where kept versions live) is left out of `ls`, `search`, `starred`,
  `recent` and `trash ls`; `--all` shows it. Listing inside it on purpose (`ls .craftcloud/versions`) shows it too.
- **Nothing secret is printed**: no tokens, ever. A share link's URL is the link itself.
- **Errors** are one line in plain words on stderr (`craftcloud: Documents/a.txt: no such file or folder`), with
  cloud paths, never the server's own (`/Personal/…`); with `--json`, a JSON error (see "JSON output"). Every
  command exits with a code from the table under "Exit codes".
- **Times** are shown in this computer's time zone (`2026-11-13 21:54`). In `--json` they are RFC 3339; where the
  local time matters (`versions`) with this computer's offset, elsewhere in UTC.

### The same option means the same thing everywhere

| Option | Meaning |
|---|---|
| `--json` | Output for scripts (see "JSON output"); any command, before or after it (also with a link) |
| `--server URL` | Another server than the desktop client's (or `CRAFTCLOUD_SERVER`); any command |
| `--all` | Also CraftCloud's own `.craftcloud` folder (`ls`, `starred`, `recent`, `search`, `trash ls`) |
| `--app APP` | Only the files a Craft app opens (`ls`, `starred`, `recent`, `search`, `files`); for `save`, the app whose folder it goes into |
| `--limit N` | At most `N` results (`search`) |
| `-f`, `--force` | Replace what is already there (`get`, `put`, `save`); an overwritten cloud file is kept as a version first |
| `--wait` | Wait until the desktop client is done (`save`); `sync` waits unless `--no-wait` |
| `--yes` | Don't ask (`trash empty`) |
- `CRAFTCLOUD_CONFIG_DIR` points to another settings folder; `CRAFTCLOUD_KEYCHAIN=off` keeps the tokens in a file
  only you can read in that folder instead of the keychain (for servers without a desktop session).
- `RUST_LOG` sets how much goes to standard error (default `error` for a command, `warn` for the window and tray;
  `RUST_LOG=debug` or `trace` for more, also per module: `RUST_LOG=craftcloud_native=debug`). Trace logging leaves
  out HTTP bodies and headers: the HTTP library (`ureq`, `ureq_proto`) logs at most at debug level (method, address,
  status), whatever `RUST_LOG` asks for. The same holds for `craftcloud-admin` in the server image.

## Commands

### Signing in

| Command | What it does |
|---|---|
| `login [SERVER] [--no-browser]` | Signs this computer in: prints a code and an address, opens the browser there, waits for you to approve the code. Without `SERVER`, the desktop client's server. If the desktop client has no server yet, this becomes its server, and the desktop client starts signed in (running already, it takes the sign-in at once): it asks which folders to sync, or syncs when a folder is set |
| `login WEBDAV-ADDRESS --kind webdav --user NAME` | *Experimental, needs `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` until 0.3.0* ([Other servers](#other-servers-experimental)). Signs in to any other WebDAV server with an app or WebDAV password you made there (never your account password), read from `CRAFTCLOUD_APP_PASSWORD` or asked for on the terminal, never an argument. `WEBDAV-ADDRESS` is the server's WebDAV base (`https://cloud.example.org/remote.php/dav/files/NAME/`); `https://` only, except to this computer. Checks the address with one listing, keeps the sign-in in the keychain, runs the check on connect and prints the sync mode it allows |
| `logout` | Ends the sign-in on the server and forgets it here (the desktop client is signed out too). It is forgotten here even when the server can't end it; then it says so, and exits with code 8 (the server can't be reached) or 1 (the server refused): the sign-in may still be active on the server until it runs out. An app password is only forgotten here (remove it on the server yourself); this also makes the settings CraftCloud's own server's again |
| `usage` | Storage used and the limit, and how much of it kept versions take (`1.1 GB of 10 GB used (200 MB in earlier versions)`). Sizes are decimal everywhere (1 MB = 1,000,000 bytes) |

### The desktop client's sync

`status`, `pause` and `resume` talk to a running desktop client (or `craftcloud daemon`) through a local control
channel; when none runs they exit with code 6. `files`, `save` and links use the same channel when a client runs,
and work without one.

| Command | What it does |
|---|---|
| `status` | The sync state, the folder, conflicts, places left out and the latest activity; the sync mode when it isn't two-way (`mode     download only …`). Exits with code 9 while the sync is held (a mass delete or a missing sync folder) or a place in a synced folder can't be read |
| `sync [--no-wait]` | Syncs now through the running client, and waits until that sync is done (unless `--no-wait`). When no client runs, it does one sync pass itself, like `sync --once` |
| `sync --confirm-deletes` | Carries out the deletions a mass-delete hold waits on (see below), then waits for that sync. It says how many (`Deleting 3000 files…` on stderr, then `Deleted 3000 files.`). A folder deleted whole goes to the cloud's trash as one item (one activity line, `Deleted t14/many (folder with 3000 files, to the cloud trash)`). With `--once`, in this process |
| `sync --discard-deletes` | Brings back the files a mass-delete hold would delete instead: downloaded again where they were deleted on this computer; where they were deleted in the cloud, put back from the cloud's trash (the same files and folders, so their links, stars and versions stay), and uploaded again only when they aren't in the trash any more |
| `sync --once` | One full sync pass in this process, with the saved settings and sign-in, then exits. A file written in the 2 seconds before is held by that pass (an app may still be saving it), so `--once` then waits those seconds once and runs a second pass for it. Refused (code 5) while the desktop client or a daemon syncs the folder |
| `daemon` | The sync engine without a window or tray, until stopped (Ctrl+C or a signal). Logs activity to stderr (JSON lines on stdout with `--json`), and answers `status`, `sync`, `pause` and `resume` like the desktop client. Refused (code 5) when one already runs |
| `pause`, `resume` | Pauses or resumes syncing |

**A mass delete is held.** When one pass would delete more than half of the synced files (at least 6), more likely
a missing drive or a mistake than intent, those deletions wait and everything else keeps syncing. `status` then
says `mass_delete`, lists the files (`pending_deletes` in `--json`) and exits with code 9, until the desktop
client, `sync --confirm-deletes` or `sync --discard-deletes` decides. While it waits, the folders those files are in
still count as synced: no pass makes them again (empty) on the side they were deleted from.

One sync engine per folder: the desktop client, a daemon and `sync --once` each claim the sync folder
(`<folder>/.craftcloud/engine.lock`) before syncing, and the second one says so and stops. A folder another sync
program keeps its database in (`.sync_*.db`, `.owncloudsync.log` or `.csync_journal.db`: Nextcloud's or ownCloud's
desktop client) is refused the same way (code 5): two sync engines on one folder fight over it. The sync folder and
the folders to sync are chosen in the desktop client.

How the control channel works: the client listens on a free port on 127.0.0.1 and writes the port and a random
secret to `control.json` in its settings folder, readable only by you. The command line sends the secret with
each request. This works the same on Linux, macOS and Windows.

### Files and folders

| Command | What it does |
|---|---|
| `ls [PATH] [--app APP]` | Lists a folder (default: the top). Size, last change, name; folders end in `/`, starred items have `*` |
| `get REMOTE [LOCAL] [-f]` | Downloads a file into the current folder, into `LOCAL` when it is a folder (or ends in `/`), or as `LOCAL`. `-` writes to stdout. A folder downloads as `NAME.zip`. An existing local file is kept unless `-f` |
| `put LOCAL [REMOTE] [-f]` | Uploads a file to the top, into `REMOTE` when it is a folder (or ends in `/`), or as `REMOTE`. An existing file is kept unless `-f`; with `-f` the old one is kept as a version first (see `versions`). If the file changed on the server meanwhile, nothing is written (code 5) |
| `mkdir [-p] PATH` | Makes a folder; `-p` makes missing parents and doesn't mind if it exists |
| `mv FROM TO` | Moves or renames; into `TO` when it is a folder (or ends in `/`). Never overwrites. Its kept versions go along |
| `cp FROM TO` | Copies a file or a whole folder, the same way |
| `rm PATH…` | Moves files and folders to the trash |
| `trash ls` | What is in the trash, and when it goes for good |
| `trash restore ITEM [--as PATH \| --rename]` | Puts an item back where it was. `ITEM` is its old path, its name (when only one item has it), or its id from `trash ls --json`. When something else is there now, it is refused (code 5) unless `--rename` (restored next to it as `NAME (restored).EXT`, `(restored 2)` and so on) or `--as PATH` (restored there; ids, links and stars stay) |
| `trash rm ITEM` | Deletes one item for good, and its kept versions (unless a file is at its old place again: then the versions are that file's) |
| `trash empty [--yes]` | Deletes everything in the trash for good, with the kept versions of what was in it (as for `trash rm`); asks first unless `--yes` (without a terminal, `--yes` is required) |
| `star PATH`, `unstar PATH` | Stars or unstars a file or folder |
| `starred [--app APP]` | Starred files and folders |
| `recent [--clear] [--app APP]` | Recently opened files, newest first (or clears the list) |
| `search QUERY [--type KIND] [--in FOLDER] [--limit N] [--app APP]` | Searches names. `KIND`: folder, image, video, audio, document, archive, code, font |
| `open PATH` | Opens the synced local copy with the app your system picks (it must be in a synced folder, and synced; a symlink out of the sync folder is refused). On Windows, a file whose type has no app yet (`.md` or `.docx` on a fresh Windows) gets the "Open with" dialog |
| `craftcloud://open?path=…`, `craftcloud://reveal?path=…` | A link as the only argument (how the system starts CraftCloud for one): see "Links" below |

### The Craft apps

What the ArtCraft Launcher and the Craft apps call to list and save CraftCloud files without the user's sign-in
(the contract, with examples: [CRAFT-INTEGRATION.md](CRAFT-INTEGRATION.md), section 3).

| Command | What it does |
|---|---|
| `apps` | The Craft apps: id, name, and the extensions each opens. Built into the program (from `apps.toml`), so it works offline and needs no sign-in |
| `--app APP` on `ls`, `recent`, `search`, `starred` | Keeps only the files `APP` opens (by extension, any case; folders are left out). `APP` is an id from `apps`, in any case. `.md` and `.txt` are WordCraft's. `search --app` asks the server for up to 200 matches, then keeps up to `--limit` of the app's files |
| `files [--app APP] [--in FOLDER]` | The synced files on this computer. With a desktop client (or daemon) running, it answers from its local view of the sync folder: no network, works offline, fast. With none running (or one that isn't signed in), it lists the cloud instead, folder by folder (slower: one request per folder) |
| `save FILE [--to FOLDER] [--app APP] [-f] [--wait]` | Saves a local file into CraftCloud, into `FOLDER`, or else into a folder named after the app (`--app`, or the app that opens the file by default): `notes.md` goes to `WordCraft/notes.md`. A file with no default app needs `--to`. With a client running and the folder synced, the file is copied into the sync folder (whole, through a temporary name the sync ignores) and the client uploads it; `--wait` waits until it has (up to 30 minutes). With no client, or a folder this computer doesn't sync, it is uploaded directly, making the folder first. An existing file is kept unless `-f`; with `-f` the old one is kept as a version first, as for every overwrite. Through the client, "existing" means on this computer: a file that is only in the cloud so far (not downloaded yet) isn't refused, and the sync then keeps both as a conflict copy |

### Share links

| Command | What it does |
|---|---|
| `share create PATH [--expires WHEN] [--password]` | Makes a link and prints it. `WHEN`: a date (`2027-01-31`, the link works to the end of that day, UTC), a time (`2027-01-31T18:00:00Z`) or a duration (`12h`, `7d`, `2w`) |
| `share ls [PATH]` | Your links, or the links to one file or folder, each with where its file or folder is now (looked up by its id, one request per link; a link whose item is gone shows the name it was shared with) |
| `share rm LINK` | Removes a link, by its id, its URL, or every link to a path |

OxiCloud can't remove a link's expiry once it has one; make a new link instead. The web's "Share link" sheet
reads expiries by the same rules (its 1, 7 and 30 days are `1d`, `7d` and `30d`), except that a day picked there
lasts to the end of that day in the browser's time zone rather than in UTC. The web's "Your share links" page
(`/links`, from the menu under your name) lists links as `share ls` does and revokes them as `share rm` does; there a
link whose item is in the trash or deleted is "No longer there", and one whose lookup failed otherwise is not
counted as gone.

### Versions

Before a file is overwritten (by the desktop client, `put -f` or a restore), CraftCloud keeps the old one in
`.craftcloud/versions/<path>/` on the server, the newest 5. Versions follow the file when `mv` (or the desktop
client) renames or moves it, and go when it is deleted for good. A new file made where another one was (`put`,
`save`) starts with no versions: what was kept at that path belonged to the earlier file, and is deleted then. A version is named after when it was replaced and when
its content was last changed (`20261113T151013.123Z_20261113T140205Z`): the server gives a copy the time of the
copy, so the content's own time is kept in the name.

| Command | What it does |
|---|---|
| `versions PATH` | The kept versions, numbered from 1 (the newest): when each one's content was last changed, its size, and when it was replaced, in this computer's time zone. Versions kept before the content's time was recorded (or kept by the web frontend) show `-` there |
| `versions PATH restore N` | Puts version `N` back; the version it replaces is kept too, so a restore can be undone |

### The server

For everyone, not only admins.

| Command | What it does |
|---|---|
| `server check` | What the server guarantees the sync, each check in plain words (a create that refuses to overwrite, an overwrite that refuses a stale version tag, a move that never lands on an existing name, version tags that change on every write, where deletes and replaced bytes go, the names it can't take …), and the sync mode that allows: **two-way**, **download only** or **refused** ([WEBDAV-STORAGE.md § 7.3](WEBDAV-STORAGE.md)). On a CraftCloud server these are *known*, not measured: nothing is written to the server and no sign-in is needed, only that it answers. Exit code 8 when it can't be reached. On another WebDAV server ([Other servers](#other-servers-experimental)) it runs the check on connect afresh: twenty steps in `.craftcloud/probe/<device>-<random>/` on the server (guarded writes only, cleaned up at the end), each printed with what was found, and the result saved for the sync (`storage-check.json` in the settings folder) |

### Other servers (experimental)

*Behind `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` until 0.3.0* ([WEBDAV-STORAGE.md § 16](WEBDAV-STORAGE.md)): without it, `--kind`
isn't in the help, `--kind webdav` is a usage error, and a settings file naming another kind of server is refused
with "This build syncs only with a CraftCloud server; set CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 to try other servers"
(code 1) by every command but `logout` and `apps`, nothing written. Nothing in a release, its packaging or its
settings sets the variable.

With it, `craftcloud login https://cloud.example.org/remote.php/dav/files/john/ --kind webdav --user john` signs in
to any WebDAV server (Nextcloud, ownCloud, a NAS …) as "another WebDAV server": the sync keeps its own trash
(`.craftcloud/trash/<time>-<device>/<path>` on the server, a `MOVE`, never a delete) and its own versions
(`.craftcloud/versions/`) there, as it does on CraftCloud's server. The check on connect decides the mode: two-way
where the server honours every write guard, **download only** where it doesn't (nothing is written to the server;
changes made here stay here and are listed as `not synced … (not uploaded)`; a file deleted in the cloud goes to
`.craftcloud/trash/` in the local folder), refused where a file's version tag can't be trusted. `ls`, `get`, `put`,
`save`, `mkdir`, `mv`, `rm`, `files`, `open`, `sync`, `daemon`, `status` and `server check` work; `share`, `star`,
`starred`, `recent`, `search`, `usage`, `cp`, `server info|users|settings`, and for now `trash` and `versions`, answer
"not available on this server" (code 10). The desktop client syncs CraftCloud's own server only so far: with such
settings its window opens and says to use `craftcloud daemon` or `sync`. Nextcloud, ownCloud and OpenCloud get
their own profiles (their trash, versions and browser sign-in) in later pull requests; until then
`--kind nextcloud|owncloud|opencloud` is refused.

### Server administration (owners and admins)

| Command | What it does |
|---|---|
| `server info` | The server's address, CraftCloud version, public URL, whether the Craft apps run in the browser, and who you are signed in as |
| `server users ls` | People, their role, storage and whether they can sign in |
| `server users add NAME --email EMAIL [--quota SIZE] [--admin]` | Adds a person (an admin with `--admin`); asks for their password |
| `server users passwd NAME` | Sets a new password; asks for it |
| `server users enable NAME`, `disable NAME` | Lets a person sign in, or stops them |
| `server users quota NAME SIZE` | Their storage limit: `10GB`, `500MB`, `1.5TB`, `512MiB`, or `unlimited` |
| `server settings show` | Whether the Craft apps run in the browser, and which are switched off |
| `server settings set [--web-apps on\|off] [--disable-app APP]… [--enable-app APP]…` | Changes those settings (the Craft apps need the `-apps` image) |

## Links

`craftcloud 'craftcloud://open?path=/Documents/Q3%20report.docx'` (or `reveal`) is what the system runs when a
`craftcloud://` link is clicked (docs/CRAFT-INTEGRATION.md, section 2). It never opens a window:

1. If a desktop client or `craftcloud daemon` runs, the link goes to it over the control channel; it checks the
   link against its own sync folder and opens the file. This process exits with what it answered.
2. Otherwise (none runs, or it isn't signed in) this process checks the link against the saved settings, opens the
   file and exits.

Global options may come before or after the link (`craftcloud --json 'craftcloud://…'`); `--json` prints
`{"opened": local path}` (on Windows a file type with no app gets the "Open with" dialog, as for `open`, but a
link's output has no `with`), and `--server` changes nothing (a link opens the copy in the desktop client's sync folder). Exit codes: 0 opened; 2 a bad link (not `open` or
`reveal`, no `path`, `..`, a `.craftcloud` path) or no sync folder yet; 4 the file is in a folder this computer
doesn't sync, leads outside the sync folder (a symlink), or isn't downloaded yet; 1 the system couldn't open it.
When stderr isn't a terminal (a link clicked in a browser), the message is also shown as a desktop notification on
Linux (`notify-send`, when installed); on macOS and Windows it only reaches the log.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | It worked |
| 1 | Something else went wrong (the message says what) |
| 2 | The command line is wrong (unknown command or option, a bad value) |
| 3 | Not signed in, or the sign-in ran out: run `craftcloud login` |
| 4 | Not found: no such file, folder, version, link or person |
| 5 | Conflict: something is already there, it changed meanwhile, or another sync engine has the folder |
| 6 | No desktop client (or daemon) is running |
| 7 | Not allowed: the server refused (e.g. an admin command without admin rights) |
| 8 | The server can't be reached |
| 9 | The sync is held and waits for you: a mass delete (`sync --confirm-deletes` or `--discard-deletes`), a missing sync folder, or a place in a synced folder that can't be read |
| 10 | Not available on this server: a command CraftCloud's own server has and this kind of server hasn't (share links, stars, search, people, the server's settings) |

## JSON output

With `--json`, a command prints one JSON value on stdout (`login` prints two lines, `daemon` one line per event),
and an error prints `{"error": {"kind": "not_found", "message": "…", "exit_code": 4}}` on stderr, `kind` being
`error`, `usage`, `signed_out`, `not_found`, `conflict`, `no_client`, `forbidden`, `unreachable`, `held` or `unsupported`. Fields may be
added later; existing ones keep their meaning. Times are RFC 3339 (UTC; `versions` gives them with this computer's offset) unless they are seconds since 1970 (`at`,
`last_synced`).

**Items** (`ls`, `starred`, `recent`, `search`, `files`, `trash ls`) are arrays of:

```json
{"path": "Documents/notes.md", "name": "notes.md", "type": "file", "size": 11,
 "modified": "2026-11-13T21:53:39Z", "starred": false, "id": "f7fc37c9-…",
 "local_path": "/home/john/CraftCloud/Documents/notes.md", "sync": "synced"}
```

`type` is `file` or `folder`; `size` is `null` for folders. Extra fields: `starred_at` (`starred`), `opened_at`
(`recent`), `trashed_at` and `expires_at` (`trash ls`), `score` (`search`, higher is better).

For files (not in `trash ls`), `local_path` is the synced copy on this computer, or `null` when it isn't here
(a folder this computer doesn't sync, not downloaded yet, no sync folder, or `--server` names another server).
`sync` says where the file stands here: `synced` (the same as in the cloud), `pending` (new or changed on one side;
the next sync moves it), `syncing`, `error`, `conflict` (only a running client knows these three), or `excluded`
(in a folder this computer doesn't sync). It is left out when there is no sync folder. Both come from the disk and
the sync's own records, without asking the server. `files` answered by a running client has no `id`, and `starred`
is always `false` there (the client doesn't know stars).

**Status** (`status`, `pause`, `resume`, `sync` through a client):

```json
{"syncing": true, "server": "https://cloud.example.org", "local_folder": "/home/john/CraftCloud",
 "state": "up_to_date", "files_done": 0, "files_total": 0, "cycles": 12, "last_synced": 1794606853,
 "conflicts": [], "activity": [{"at": 1794606853, "kind": "up", "path": "Documents/notes.md", "size": 11, "detail": ""}]}
```

`state` is one of `idle`, `scanning`, `downloading`, `up_to_date`, `paused`, `offline`, `signed_out`,
`mass_delete`, `folder_missing`, `failed`; `error` is set for `offline` and `failed`, `pending_deletes` (the files
whose deletion waits) for `mass_delete`. `mode` is what the sync may do with the server, `two_way`, `download_only` or
`refused` ([WEBDAV-STORAGE.md § 7.3](WEBDAV-STORAGE.md); CraftCloud's own server is `two_way`), once the first pass has run. `not_synced` lists places in synced folders that the last pass left out, as `{"path", "reason"}`; `reason` is `unreadable` (a folder or a file that can't be read, such as one without read permission: it, and anything under it, isn't synced or deleted until it can be), `symlink` (a symbolic link, never followed), `name` (a name the cloud can't take, or one this computer can't have: on Windows, `< > : " | ? *`, a trailing dot or space, a reserved name such as `CON.txt`, or two cloud names that differ only in upper and lower case; it syncs once renamed, and `status` says which), `temporary` (a FILE whose name looks like an app's temporary, lock or backup file: `*.tmp`, `*.part`, `*~`, `#*#`, `.#*`, `~$*`, `*.swp`; folders with such names sync normally) or `not_uploaded` (new or changed here under download only: it stays here as it is), and `activity` says more. It doesn't change `state`. A place that can't be read needs you, so `status` (and `sync` through a client, and `sync --once`) then say "Up to date · N places can't be read" and exit with code 9; symbolic links, names and temporary files are left out for good and don't change the exit code. `status` prints each as `not synced PATH  (why)`. `no_room` lists files that don't fit in the cloud (its quota is full); they are tried again once there is room, the
headline says "Up to date · N not synced (no room …)", and the exit code doesn't change. `waiting` lists files the last pass held
because they were written in the 2 seconds before it (an app may still be saving them); the next pass syncs them, the
headline says "Up to date · N files are still being saved", and neither `state` nor the exit code changes. `syncing` is `false` (and
`state` `signed_out`) while the client waits for a sign-in. `activity`
holds the latest 10 events; `kind` is `up`, `down`, `deleted`, `conflict`, `error`, `moved` (renamed or moved, `detail` says from where) `skipped` (left out on purpose, e.g. a folder switched off, or a name the cloud can't take) or `warning` (synced, but e.g. a name a Windows computer can't have).

**`sync --once`**: `{"state", "mode", "error", "pending_deletes", "files_done", "files_total", "conflicts", "not_synced", "waiting", "activity"}`,
with every event of that run. When the first pass held a file being saved, a second pass follows 2.5 seconds later and
the result is that pass's (`files_done` and `files_total` count it alone; `activity` has both). `waiting` lists what is
still being saved after it (empty when nothing waits): a script tells "synced" from "a file still waits" by it. It
exits 0 when the run ended up to date (with or without `waiting`), 3 when signed out, 8 when offline, 9 when held
(or a place can't be read), 1 otherwise.

**`daemon --json`** prints one line per event: `{"event": "started", "local_folder": …}`,
`{"event": "state", "state": …, "error": …}`, `{"event": "activity", "activity": {…}}`.

**Other commands:**

| Command | Prints |
|---|---|
| `login` | `{"user_code", "verification_uri", "expires_in"}`, then `{"signed_in": true, "server", "username", "default"}` (`default`: it is the desktop client's server). With `--kind webdav`: one line, `{"signed_in": true, "server", "dav_url", "kind": "webdav", "username", "mode", "refused_because", "default"}` |
| `logout` | `{"signed_out": true, "server", "was_signed_in", "server_session_ended"}` (`server_session_ended`: the server ended the sign-in, or had already; `false` when it wasn't signed in here or the server couldn't end it, with exit code 8 or 1 in that last case) |
| `usage` | `{"username", "used_bytes", "quota_bytes", "versions_bytes"}` (`quota_bytes` 0 = no limit; `versions_bytes`: what kept versions take, part of `used_bytes`) |
| `get` | `{"path", "saved_to", "size"}` |
| `put` | `{"path", "size", "replaced"}` |
| `apps` | `[{"id", "name", "opens", "default", "experimental"}]` (`default`: the extensions this app opens when a file is opened by its type; each extension has one) |
| `save` | `{"path", "local_path", "via", "state", "replaced", "size"}`: `via` is `client` (copied into the sync folder; `state` `queued`, or `synced` with `--wait`) or `server` (uploaded directly; `state` `uploaded`, `local_path` `null`) |
| `mkdir` | `{"path", "created"}` |
| `mv`, `cp` | `{"from", "to"}` |
| `rm` | `{"trashed": [paths]}` |
| `trash restore` / `rm` / `empty` | `{"restored", "from", "id"}` (`restored`: where it is now, `from`: where it was) / `{"deleted", "id"}` / `{"emptied": true}` |
| `star`, `unstar` | `{"path", "starred"}` |
| `recent --clear` | `{"cleared": true}` |
| `share create`, `share ls` | a link, or an array of `{"id", "url", "type", "item_id", "path", "name", "expires_at", "has_password", "created_at", "access_count"}` (`path`: where the item is now, `null` when it is gone; `name`: its name, or the one it was shared with) |
| `share rm` | `{"removed": [ids]}` |
| `versions` | `[{"n", "stamp", "time", "modified", "replaced_at", "size"}]`: `modified` is when the content was last changed (`null` when not recorded; never after `replaced_at`), `replaced_at` when it was replaced, both ISO 8601 in this computer's time zone with the offset (`2026-11-13T14:02:05+01:00`); `time` is the old replacement time text (UTC), kept for scripts |
| `versions … restore` | the restored version's fields, with `path` and `restored` (its `stamp`) |
| `open` | `{"opened": local path}`, with `"with": "chooser"` when Windows had no app for the file's type and showed its "Open with" dialog instead |
| `server info` | `{"server", "craftcloud", "version", "public_url", "web_apps", "initialized", "signed_in_as", "role"}` |
| `server check` | `{"server", "kind": "oxicloud" \| "webdav", "mode": "two_way" \| "download_only" \| "refused", "measured", "checks": [{"check", "result"}]}` (`measured`: `false` when the capabilities are known for the kind of server rather than probed; `checks`: one row per capability, in plain words). On another WebDAV server also `"checked_at"`, `"refused_because"` and `"steps": [{"step", "number", "what", "result", "ok"}]`, one per step of the check |
| `server users ls` | `[{"username", "email", "role", "active", "used_bytes", "quota_bytes", "last_login_at", "id"}]` |
| `server settings` | `{"web_apps", "disabled_apps", "bundled_apps"}` |

## Windows

The desktop client, `craftcloud.exe`, is a GUI program on Windows (no console window opens with it), so Windows
gives it no console and on its own its output can't reach a terminal (attaching to one needs `unsafe`, which this
workspace forbids). Next to it the installer and the portable zip put **`craftcloud.com`**, a small console program
(`apps/craftcloud-cli`) that runs `craftcloud.exe` with the same arguments, passes the input and output through,
and exits with its exit code. Command Prompt and PowerShell try `.com` before `.exe`, so `craftcloud status` typed
in a terminal runs it. The Start menu shortcut, start at login and `craftcloud://` links run `craftcloud.exe`.

- **PATH:** the installer adds its folder to your PATH (for you only; uninstalling takes it off again). A terminal
  opened before the install doesn't see it yet: open a new one. From the portable zip, run it in that folder
  (`.\craftcloud status` in PowerShell, `craftcloud status` in Command Prompt) or add the folder to PATH yourself.
- `craftcloud` without a command opens the window and gives the terminal back at once.
- **Ctrl+C** stops the command. Windows sends it only to programs attached to the console, which `craftcloud.exe`
  isn't, so `craftcloud.com` stops it; the exit code is then Windows' usual one for Ctrl+C (`-1073741510`).
- Questions still work (passwords, `trash empty`'s "[y/N]"): the input reaches `craftcloud.exe` through a pipe, and
  `craftcloud.com` tells it a person is typing (`CRAFTCLOUD_STDIN_CONSOLE=1`). A password shows as you type, as
  it always did on Windows.
- Output sent to a file or another program (`craftcloud ls > files.txt`, `craftcloud ls | findstr txt`) is handed on
  unchanged, as it was before. A program that starts `craftcloud` itself and reads its output (Rust's
  `Command::new("craftcloud")` looks for `craftcloud.exe`) gets the client directly; that has always worked.

## Tests

`cargo test -p craftcloud` covers argument parsing and output formatting. The end-to-end test runs every command
against a real server and is skipped unless pointed at one (use a throwaway container):

```sh
CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8093 CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD=… \
  cargo test -p craftcloud --test cli_live
```
