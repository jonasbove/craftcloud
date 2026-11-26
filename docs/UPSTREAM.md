# What CraftCloud needs from upstream

These are draft issues, ready to file. CraftCloud does not patch upstream code; it works around each gap in its own
code until a fix ships. When one ships, the fix in CraftCloud is usually a change to `apps.toml` (for example
`bridge = "native"`), and the workaround can then be removed.

The version numbers are the pins of 2026-11-12. Source references are to the upstream repositories at those pins.

## A. Craft apps (github.com/storytold)

### A1. Adopt `craft-web-host` in the web build

**Repos:** photocraft, designcraft, soundcraft, texturecraft, materialcraft (no bridge at all); vectorcraft and
effectcraft (they have an open API but no save event).

> **Support `?host=parent` (craft-web-host) in the web build**
>
> WordCraft, GridCraft, DeckCraft, PdfCraft, FigCraft, CADCraft and FilmCraft already embed `craft-web-host`
> (craft-libs `crates/web-host`). A same-origin page can then frame the app with `?host=parent`, send
> `<app>:open {name, bytes}`, and receive `<app>:save`, `<app>:dirty`, `<app>:ready` and `<app>:failed`.
> `<app>` web builds don't do this yet. File › Open uses the rfd picker, and Save builds an `<a download>`
> link. So a host such as a self-hosted cloud or a CMS can't hand the app a file or get the saved bytes
> back without injecting scripts that fake a drop event and intercept the downloads.
>
> Ask: depend on `craft-web-host` at the shared tag, then:
> - route the app's write and download service through `Host::post_save` when a host is connected;
> - feed `open` messages into the same path as File › Open;
> - report dirty-state changes.
>
> Builds with no host keep working unchanged.

Notes per app:

- **photocraft:** when a document is already open, a dropped file is *placed* in it. An `open` message should open it
  as a new document.
- **soundcraft:** an `open` of an audio file should create a new session that contains the file, not import it into
  the demo session.
- **vectorcraft:** `vectorcraft.openFile` already exists. Only `vectorcraft:save` is missing, so the host doesn't
  have to poll `captureDownloads` / `files()`.
- **effectcraft:** `effectcraft.addFile` already exists. Only `effectcraft:save` is missing.
- **designcraft:** opening a `.designcraft` or `.idml` file through the bridge must open it as a document, not place
  it as an image.

### A2. Wait for the host to confirm a save before clearing dirty

**Repos:** craft-libs (web-host), plus every app that embeds it.

> **Send `id` with `<app>:save` and clear dirty only after `<app>:saved`**
>
> In current releases no app sends an `id` with `save` (e.g. wordcraft `apps/wordcraft-web/src/host.rs:98`,
> filmcraft `host.rs:144`). Each app marks the document clean as soon as it posts the bytes. If the host's
> upload then fails (offline, quota, permission), the user sees a clean document while the change exists
> nowhere.
>
> Ask:
> - attach an `id` to every save;
> - keep the document dirty until the host replies `{type:"<app>:saved", id, ok:true}`;
> - show the host's error when it replies `ok:false`.

### A3. Report `saveAs` correctly

**Repos:** gridcraft, cadcraft.

> **`<app>:save` always reports `saveAs:false`**
>
> GridCraft (`apps/gridcraft-web/src/host.rs:15-18, 66`) and CADCraft (`host.rs:10-22`) send `saveAs:false`
> for Save, Save As and exports alike. A host therefore can't tell "overwrite the open file" from "this is
> a new file", and has to guess from the file name.
>
> Ask: set `saveAs:true` for Save As and for exports, as DeckCraft and FigCraft do.

### A4. Route exports through the host

**Repo:** pdfcraft.

> **Exports download even in host mode**
>
> When the app is hosted, Save and Save As go to `pdfcraft:save`. But exports, print-to-PDF, OCR output and
> CSV still trigger a browser download (`crates/ui-egui/src/editing/save.rs:461-476` and its callers).
>
> Ask: when a host is connected, send these to the host with `saveAs:true`, as the other bridged apps do.

### A5. Keep the service worker out of the host's URLs

**Repo:** effectcraft.

> **`sw.js` caches every same-origin GET from its controlled pages**
>
> `web/sw.js:30-48` serves every same-origin GET cache-first. When EffectCraft runs inside a larger site,
> files fetched from that site (for example a cloud file the user just changed) are served stale from the
> cache.
>
> Ask: cache only URLs under the app's own scope. CraftCloud loads the app with `?nosw` until this is fixed.

### A6. LightCraft: align with `craft-web-host`

**Repo:** lightcraft.

> **Use the shared bridge messages (`saved`, `handoff`, `id`)**
>
> LightCraft has its own `lightcraft:open` / `lightcraft:save` / `lightcraft:cmd` bridge
> (`apps/lightcraft-web/src/host.rs`). Using the shared `craft-web-host` messages, including A2, would let
> hosts treat it like the other apps.
>
> Also useful for cloud hosts: a way to write a developed photo back next to its original. Today edits stay
> in the browser's storage (OPFS / IndexedDB) and only exports leave the app.

### A7. Publish a web build

**Repo:** anicraft.

> **No `anicraft-web-<ver>.zip` release asset**
>
> `docs/web.md` says "Nothing is hosted yet". Releases v0.1.0–v0.2.1 have no web asset, unlike every other
> Craft app.
>
> Ask: attach the web zip produced by `scripts/build-web.sh` to releases.

### A8. Optional: open a file from a URL

**Repos:** all apps except pdfcraft (which already supports `?file=<url>`).

> **Support `?file=<url>` in the web build, as PdfCraft does**
>
> A same-origin or CORS URL would be fetched and opened at startup. A link could then open a file in the
> app with no host script at all.

### A9. Projects that reference media by path

**Repos:** filmcraft, effectcraft.

> **Media in projects is referenced by absolute path**
>
> `.fcproj` (`docs/project-files.md:21`) and `.ecproj` store media paths, so a project opened in the browser
> from a cloud folder can't find its media.
>
> Ask: either
> - store paths relative to the project file and let a host resolve them (e.g. `<app>:need {path}` →
>   `<app>:open`), or
> - offer a "packaged project" format that embeds the media, like soundcraft's `.scraftz`.

### A10. Later: live co-editing

**Repos:** craft-libs plus the document apps.

> **Expose an operation stream for collaborative editing**
>
> The host bridge moves whole files. Real-time co-editing would need each app to emit and apply document
> operations, for example a Yjs/yrs-compatible update stream over `craft-web-host`. That would let a host
> relay them between users the way OxiCloud already relays its Markdown editor. This is a long-term request
> and CraftCloud does not depend on it.

### A11. PhotoCraft 0.23.1 web zip is missing `app/snippets/`

**Repo:** photocraft.

> **Web build hangs on "Loading PhotoCraft…": `app/snippets/…/inline0.js` is 404**
>
> `app/photocraft-web.js` imports `./snippets/eframe-d31a8450ea12dfc1/inline0.js`, which resolves to
> `app/snippets/…`. `photocraft-web-0.23.1.zip` only ships `snippets/` at the top level, so the import
> fails and the page reaches its "stalled" state on any host. Copying `snippets/` into `app/` fixes it.
> The page also requests `fonts/manifest.txt`, which the zip doesn't contain (404, not fatal).
> `HOSTING.md` gives the part size as 10.9 MiB; the 0.23.1 part is 15.96 MB.
>
> Ask: package the snippets next to the module that imports them (or import them from `../snippets/`),
> and add a smoke test that loads the zip as released. CraftCloud copies the folder at build time
> (`fixups` in `apps.toml`) until this ships.

## B. craft-launcher (github.com/storytold/craft-launcher)

### B1. Machine-readable file associations in the catalogue

> **Add `extensions` / `mime` fields to `catalog.toml`**
>
> Formats appear only in prose under `features`. A list per app of the formats it opens, saves and imports
> would let the launcher, desktop packaging (MIME types in `.desktop` files, `CFBundleDocumentTypes`) and
> hosts such as CraftCloud share one source of truth.

### B2. Brand licence: clarify "Craft App Suite" and the app names

> **Brand licence: is "Craft App Suite" covered, and may third parties use app names factually?**
>
> `docs/brand/LICENSE-brand.txt` covers the ArtCraft name, wordmark and mark. It does not say whether
> "Craft App Suite" or the app names (PhotoCraft, …) are marks. Third-party hosts that bundle the
> MIT/Apache apps want to name them factually, e.g. "Open in WordCraft".

## C. OxiCloud (github.com/AtalayaLabs/OxiCloud)

CraftCloud runs the upstream binary unmodified and fixes these itself, using its front proxy and frontend patches. Each
fix below would remove some of that glue.

### C1. Configurable framing headers per path

> **Allow same-origin framing for static sub-apps**
>
> `security_headers.rs` sets `X-Frame-Options: DENY` and `frame-ancestors 'none'` on every response. Static
> apps served from the static directory therefore can't be put in an iframe by OxiCloud's own SPA.
>
> Ask: one of
> - an env var such as `OXICLOUD_FRAMEABLE_PATHS=/apps/` that sends `SAMEORIGIN` / `'self'` for those paths;
> - honour a `_headers` file in the static directory.

### C2. Directory index for static sub-apps

> **`/apps/x/` returns the SPA shell instead of `/apps/x/index.html`**
>
> Static sub-directories need an explicit `index.html` in their URLs, because `append_index_html_on_directories`
> is `false`. A missing asset also returns the SPA shell with HTTP 200 rather than a 404.

### C3. File versioning

> **Keep previous versions on overwrite**
>
> An overwrite through WOPI PutFile, WebDAV PUT or delta commit replaces the content with no history
> (`docs/compare.md`: "Versioning ❌"). With content-addressed storage, keeping the last N versions should
> be cheap. Until this exists, CraftCloud keeps its own snapshots in `.craftcloud/versions/`.

### C4. Version check on WOPI PutFile

> **PutFile has no version precondition**
>
> PutFile checks only the lock (`wopi_handler.rs:293-316`). A write made in the meantime through WebDAV or
> REST is silently overwritten.
>
> Ask: honour `X-WOPI-ItemVersion` or `If-Match`, and return 409 with the current version.

### C5. Replace a file by id over REST

> **REST has no simple "replace content of file {id}"**
>
> `POST /api/files/upload` only creates files, and replacing one needs the FastCDC + BLAKE3 delta protocol.
> A plain `PUT /api/files/{id}/content` with `If-Match` would serve simple editors.

### C6. An "open with" registry in the frontend

> **Pluggable viewers and editors**
>
> `FileViewer.svelte` chooses the viewer with a hard-coded `kindOf()` plus the WOPI extension list. A small
> registry (extension or MIME → handler, with a default and alternatives) would let deployments add or
> replace editors without patching the SPA.

### C8. A folder's ETag doesn't always change when a file is added to it

> **WebDAV folder ETag / `getlastmodified` stay the same after a new child file**
>
> Seen on 2026-11-13 with v0.10.0: a `PUT` that created `Shared/notes (conflict ….md` at 17:05:30 left
> `Shared`'s `getlastmodified` at 17:05:29 and its ETag unchanged, in a listing made after that `PUT`. In an
> earlier test (with a second between the steps) the ETags did change. Clients therefore can't use folder
> ETags to skip unchanged subtrees, and must list every folder each time.
>
> Ask: update a folder's modification time (and so its ETag) whenever anything below it is created,
> changed or deleted, the way `getetag` on collections is commonly relied on by sync clients.


### C9. A share link's expiry can't be removed

> **`PUT /api/shares/{id}` can set `expires_at` but not clear it**
>
> `null` means "leave as it is" and `""` is refused (422, not RFC 3339), so once a link has an expiry it keeps
> one; the only way back to "never" is a new link (and a new URL). The password can be removed with `""`.
>
> Ask: accept an explicit way to clear it, e.g. `"expires_at": ""` or a `clear_expiry: true` flag.

### C10. Small inconsistencies in the REST answers (v0.10.0)

> - `GET /api/shares` answers `{items, pagination}`, but a bare array when `item_id`/`item_type` are given.
> - `POST /api/trash/{id}/restore` answers 200 "Item restored successfully" for an id that is not in the trash.
> - Rename and move answers (`PUT /api/files/{id}/rename|move`, batch move) carry an empty `content_hash` and an
>   `etag` like `-1794606853`, while a fresh `GET` has both.
> - `path` is `/Personal/…` in file records and search, `Personal/…` in favourites, recent and trash, and `""`
>   in `GET /api/folders/{id}/resources`.
> - WebDAV `DELETE` deletes for good while the REST deletes move to the trash; a sync client that deletes over
>   WebDAV bypasses the trash. A WebDAV client has no way to ask for the trash.
> - `POST /api/auth/refresh` with a refresh token whose session is gone (the device was removed, or the session
>   ended) answers 404 "Session not found", where a client expects 401. CraftCloud treats that 404 as signed
>   out.
>
> Ask: one shape per answer, a 404 for an unknown trash id, complete records after rename/move, one path
> convention, WebDAV deletes that go to the trash (as Nextcloud's do) or a header to choose, and a 401 for a
> refresh token that is no longer valid.

### C11. Storage use doesn't drop when the trash is emptied

> **Uploads are refused for the quota for up to 10 minutes after emptying the trash**
>
> Seen on 2026-11-14 with v0.10.0: deleting a file, emptying the trash and uploading another of the same size
> answers 507 (quota), because `users.storage_used_bytes` / `drives.used_bytes` are only recounted by the
> `usage_reconcile` job (every `OXICLOUD_STORAGE_USAGE_RECONCILE_SECS`, 600 by default). An admin can run it at
> once (`POST /api/admin/jobs/usage_reconcile/trigger`), and CraftCloud's desktop client does so for an admin;
> other people's clients try again after a few minutes.
>
> Ask: subtract the purged bytes when the trash is emptied (or an item is deleted for good), the way an upload
> adds them, or let a user recount their own use.

### C13. `If-Range` is ignored on WebDAV `GET`

> **`Range` gets a 206, but `If-Range` with a stale ETag still gets a 206 of the new bytes**
>
> Seen on 2026-11-14 with v0.10.0: `GET` with `Range: bytes=50000-` answers 206 with `Content-Range` and the ETag,
> and so does the same request with `If-Range: "nope"` (it should answer 200 with the whole file). A client that
> resumes a download must compare the 206's ETag with the one it started from itself (CraftCloud's does).
>
> Ask: honour `If-Range` (RFC 9110 13.1.5).

### C14. Single sign-on failures that end on a raw error page

> **The OIDC callback answers 400 in plain text when the provider sends `?error=…`, and `authorize` answers a
> JSON 500 when the provider can't be reached**
>
> Seen on 2026-11-14 with v0.10.0 and a Keycloak test realm:
> - When the person cancels or the provider refuses them, the provider redirects to
>   `/api/auth/oidc/callback?error=access_denied&state=…` without `code`. `OidcCallbackQueryDto` requires `code`,
>   so axum answers 400 "Failed to deserialize query string" as `text/plain`, mid-navigation.
> - `GET /api/auth/oidc/authorize` with an issuer URL the server can't reach answers 500
>   `{"error":"An internal error occurred…"}`. Saving that issuer through `PUT /api/admin/settings/oidc` was accepted
>   without a check.
>
> CraftCloud's Caddyfile turns both into redirects to `/login?login_error=provider_denied|provider_error|
> provider_unreachable|callback_failed`. Ask: redirect to `<frontend>/login?login_error=<key>` from these paths
> too (the callback already does for its own failures), and keep the provider's `error` for the log.

### C15. Several different sign-in refusals share `callback_denied`

> **A wrong client secret, a replayed state and "no account, and accounts aren't created at sign-in" all come
> back as `login_error=callback_denied`**
>
> Seen on 2026-11-14 with v0.10.0: the token endpoint's 401 ("Token exchange failed with status 401") and
> `auto_provision=false` for an unknown person ("Auto-provisioning is disabled") are both `AccessDenied`, so the
> sign-in page can't tell the person "ask an admin for an account" apart from "try again". The reasons are only in
> the container log.
>
> Ask: distinct keys, e.g. `account_not_provisioned` and `client_auth_failed` (the latter is an admin problem,
> the former a user one).

### C17. `getlastmodified` is rounded to the nearest second, `Date` truncated

> **A file written at 16:11:13.66 says `getlastmodified` 16:11:14 in a `PROPFIND` answered with `Date` 16:11:13**
>
> Seen on 2026-11-16 with v0.10.0: `PUT`s at .32 and .66 past the same second came back as that second and the
> next. So a file's time can be up to half a second after the moment it was written, and after a client's own
> clock reading taken just after the write. CraftCloud never shows a kept version's content as changed after it
> was replaced (`versions::Version::modified_at`), which also covers a server clock that runs ahead.
>
> Ask: truncate `getlastmodified` (and `Last-Modified`) to the second, as the `Date` header is.

## D. craft-common-actions (github.com/storytold/craft-common-actions)

CraftCloud builds its desktop releases with its own `.github/workflows/release.yml`, which makes the same files
as the Craft apps' releases. These would let it call the shared workflow instead.

### D1. The release workflow can't run from a repository on a personal account

> **`release.yml`: a macOS runner input, GPG signing independent of platform signing, per-target switches**
>
> Read on 2026-11-14 from `main` (no run made):
>
> - The `macos` job always runs on `macos-15-xlarge`; only `dry-run: true` skips it, and a dry run makes no
>   release. Larger runners are only for organisations on the GitHub Team or Enterprise Cloud plans, so from a
>   personal account's repository that job never gets a runner, and the `release` job, which needs it, never runs.
> - The Linux signing key (`LINUX_SIGNING_KEY`) is imported only on the `release` branch, and on that branch the
>   macOS and Windows jobs fail without all the Apple secrets and a Windows certificate. A project without Apple
>   and Windows code signing therefore can't publish GPG-signed Linux files or a signed `SHA256SUMS.txt`.
> - The `flatpak` job and the Windows arm64 build have no switch (`has-arch`, `has-freebsd` and `has-riscv64`
>   exist), and Linux aarch64 can't be left out either.
> - The release notes are GitHub's generated notes; there is no way to use the project's own release notes as the
>   body, and no build-provenance attestation.
>
> Ask:
> - a `macos-runner` input (default `macos-15-xlarge`), so `macos-15` or `macos-latest` can be passed;
> - GPG signing whenever `LINUX_SIGNING_KEY` is set, on any ref, independent of Apple and Windows signing (with a
>   `require-platform-signing` input for the projects that want the current strictness);
> - per-target switches (`has-flatpak`, `has-windows-arm64`, `has-linux-aarch64`), as `has-arch` already is;
> - optionally, a `notes-file` input and an attestation step.

## E. Nextcloud (github.com/nextcloud/server)

CraftCloud syncs with Nextcloud through its WebDAV, trash bin, versions and OCS endpoints, unmodified; the Nextcloud
profile is `crates/native/src/storage/profile/nextcloud.rs`. Nothing below is filed.

### E1. A file overwritten within the same second keeps its ETag

> **Two `PUT`s to one file within one second answer the same ETag, so `If-Match` can't tell them apart**
>
> Seen on 2026-11-17 and 2026-11-18 with Nextcloud 31.0.14 (and ownCloud 10.15), with `curl`: a `PUT` that
> overwrites a file within the same second as the write before it answers, and lists, the ETag the earlier write
> had (3 of 3 rounds). The file cache derives the tag from the storage's modified time in whole seconds, so two
> writes in one second look like one version. A second client that writes right behind the first gets the first
> client's tag; the first client's next `PUT` with `If-Match` on that tag then succeeds and overwrites the second
> client's bytes, which a precondition is there to prevent.
>
> The versions have the same grain: a version is named by its second, and an overwrite within the same second as
> the write before it leaves **no** version of the earlier bytes (6 tries on 2026-11-18: whenever the two writes
> shared a second, the ETag stayed and the versions listing had one entry). So on Nextcloud "the old bytes are
> kept before an overwrite" holds only for writes a second apart. CraftCloud's sync never writes a file twice
> within a second (a cycle lists first); `craftcloud put --force` right after `put` would, and the version it
> loses is the bytes the person just uploaded.
>
> It cuts both ways. A write right behind ours hides under our tag (above). And a write right behind one we
> already synced hides under the tag we saved: a listing shows the same ETag, so a sync client sees no change,
> never downloads it, and its own next overwrite passes `If-Match`.
>
> CraftCloud works around the ETag part (docs/WEBDAV-STORAGE.md § 16, Q13). For the second direction it reads a
> file's version as its ETag plus the checksum the server lists (a write without `OC-Checksum` clears the listed
> checksum even when the ETag stays, and a `GET` answers the stored one). For the first, every `PUT` carries `OC-Checksum: SHA1:…`, the
> file is read back afterwards, and a listed checksum that isn't ours is taken as someone else's write and kept
> as a conflict copy. That costs one `PROPFIND` per write and relies on the server storing the checksum it was
> sent (which it does, without checking it: a wrong `OC-Checksum` is accepted and listed as given).
>
> Ask: give a file a fresh ETag on every write (a counter or a hash besides the mtime), as the ETag's contract
> in RFC 7232 § 2.3 expects, and key versions finer than the second (or keep a same-second version under a
> distinct name); or, failing that, verify `OC-Checksum` against the bytes received on a plain `PUT`, as the
> chunked upload does, so a stored checksum is trustworthy.

## F. craft-libs (github.com/storytold/craft-libs)

The Craft apps' shared libraries. CraftCloud pins some of them (`craft-dav`, `craft-xml`, `craft-settings`) and
proposes the drafts below; nothing here is filed.

### F1. A `CloudBackend` that runs `craftcloud`

> **A `craft-cloud-ui` backend for CraftCloud, through its command line**
>
> The apps' Open, Save and Share pages (`craft-cloud-ui`) reach a cloud only through a `CloudBackend`.
> `craft-cloud-local` is the reference backend; no WebDAV or Nextcloud backend exists yet (step 2 of RFC-0024 in
> storytold/craft-contrib, which #2179 notes has not started). #2452, Google Drive as a `CloudBackend` of its own,
> is the precedent for one provider's backend.
>
> Proposal: a backend that runs the `craftcloud` program with `--json`, the contract in CraftCloud's
> docs/CRAFT-INTEGRATION.md § 3, and maps each `Job` onto one command (§ 5 there has the table: `List` to
> `files --json`, `Read` to the bytes on stdout, `Write` to `save --if-match <etag>`, `Create` to a `save` that never
> replaces, `Account` and `SignIn` to `status --json` and `login`). It holds no token and speaks no HTTP, so the
> sign-in, the kept versions and the write guards stay in the client. A refused `--if-match` becomes
> `Fault::Conflict`. It reports no conversation, contacts or notifications.
>
> Two things it can't give the pages: a rename-stable `room` (WebDAV has no file id, so it is the path), and a
> view or edit `Permission` on a link (CraftCloud's links have none).
>
> CraftCloud provides the command-line half (an ETag in `files --json`, `save --if-match`, a read to stdout); the
> backend itself belongs in craft-libs, next to the others.

### F2. craft-suite-account: what a provider like CraftCloud would need

> **`craft-suite-account`: a provider kind on an account, a sign-in that doesn't switch accounts, and an account
> without its token**
>
> CraftCloud could list its sign-in in the suite's accounts, so the launcher and the apps know it is there. Three
> things stand in the way (CraftCloud's docs/CRAFT-INTEGRATION.md § 5.1):
>
> - An `Account` has `service`, `user`, `display` and `email`, but nothing says what kind of deployment it is. A
>   reader expecting one kind would take a row of another kind for its own. Ask: a `kind` (or `provider`) field,
>   and readers that skip a kind they don't know.
> - `Store::sign_in` always makes the account the active one. A second kind of account, added by its own program,
>   would change the account every app uses. Ask: a way to record an account without making it active.
> - Every row has a token in the keychain. CraftCloud refreshes its tokens under its own lock, and its server ends
>   a sign-in when a spent refresh token is used again, so a second refresher would end it. Ask: a way to record
>   an account with no token, for a provider whose apps reach it through its own program.

### F3. `craft-update` for an app outside the storytold organisation

> **`craft-update`: app identity and signing details that a non-suite app can set**
>
> Read on 2026-11-26 at `de994d8`. CraftCloud's desktop client uses only the release check (default features:
> `Http::get_api`, `parse_github_release_list`, `version::offers_update`) and never installs, because:
>
> - The macOS team id the installer checks a downloaded app's signature against is fixed in the crate (ArtCraft's),
>   so an app signed by anyone else can't be installed or self-updated on macOS.
> - `App::new` defaults `developer` to "ArtCraft" and the bundle id to `ai.storyteller.<id>`; an app outside the
>   suite must remember to override both (`developer`, `bundle_id_override`), and the Linux package detection
>   (`pacman -Q <bundle id>`) follows the bundle id too.
> - Releases are trusted on their SHA-256 sums only: the GPG `.asc` signatures and the build-provenance
>   attestations that CraftCloud's releases carry are not checked.
> - `check_for_update` reads `ARTCRAFT_LAUNCHER_PRETEND_VERSION` and only ever asks api.github.com, so the
>   offering logic can't be tested against a recorded answer from outside the crate (`check_releases_at` and
>   `offered_release` are private); CraftCloud repeats their few lines over the public functions.
>
> Ask: a `macos_team_id: Option<&str>` on `App` (none: no macOS install), a constructor without the suite's
> defaults (or a `const fn` that takes them), optional signature or attestation checks, and a public
> `offered_release(json, repo, running)`.

## G. ownCloud (github.com/owncloud/core)

CraftCloud syncs with ownCloud 10 through its WebDAV, trash bin (`trash-bin/`) and versions (`meta/…/v/`) endpoints,
unmodified; the ownCloud profile is `crates/native/src/storage/profile/owncloud.rs`. Nothing below is filed.

### G1. A file overwritten within the same second keeps its ETag, and the version of the earlier bytes is lost

> **Two `PUT`s to one file within one second answer the same ETag, and only one version is kept for them**
>
> Seen on 2026-11-17 and 2026-11-18 with ownCloud 10.15.3, with `curl`: a `PUT` that overwrites a file within the
> same second as the write before it answers, and lists, the ETag the earlier write had (3 of 3 rounds on
> 2026-11-17; on 2026-11-18 three writes right behind each other, the two in the same second sharing the tag). As
> on Nextcloud (E1) the file cache derives the tag from the storage's modified time in whole seconds, so two writes
> in one second look like one version, and a second client's write right behind the first gets the first client's
> tag; the first client's next `PUT` with `If-Match` on that tag then overwrites the second client's bytes, which a
> precondition is there to prevent.
>
> The versions have the same grain: a version is named by the modified time, in seconds, of the file it was
> (`/remote.php/dav/meta/<file id>/v/<seconds>`), so when a file is overwritten twice within one second the second
> overwrite's version lands under the same name as the first's and replaces it: of three writes "xxxx", "yyyyy",
> "zzzzzz" in a row (the first two in one second), the versions kept were the bytes before "xxxx" and "yyyyy"; the
> version holding "xxxx" was gone. So on ownCloud "the old bytes are kept before an overwrite" holds only for writes
> a second apart. CraftCloud's sync never writes a file twice within a second (a cycle lists first); `craftcloud put
> --force` right after `put` would, and the version it loses is the bytes the person just uploaded.
>
> CraftCloud works around the ETag part (docs/WEBDAV-STORAGE.md § 16, Q13): every `PUT` carries
> `OC-Checksum: SHA1:…`, which ownCloud checks against the bytes received (a mismatch answers 400, measured), the
> file is read back afterwards, and a listed checksum that isn't ours is taken as someone else's write and kept as a
> conflict copy. That costs one `PROPFIND` per write.
>
> Ask: give a file a fresh ETag on every write (a counter or a hash besides the mtime), as the ETag's contract in
> RFC 7232 § 2.3 expects, and name versions finer than the second (or keep a same-second version under a distinct
> name instead of replacing it).
