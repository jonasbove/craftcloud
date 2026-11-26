<!-- craft-house-rules:start core source=279b2c09ef2b2f0b4f5e616a47d39e29db0a1eb3 sha256=bee0ffafad196b68c76425770e344e588463bbda685ab41581fcbabb85dfc930 -->
## House rules

These apply in every Craft repository. A later rule may be stricter, not weaker. The source is `standards/house-rules-core.md` in `storytold/craft-contrib`. Propose changes there, not in a copy.

1. **A person is accountable.** Whoever runs an agent answers for what it opens and for its review comments.
2. **Check, then claim.** Before writing code, read the issue comments, search pull requests for `#N` and the keywords, and check recent commits on `main`. Comment on the issue first. Release a claim you will not finish.
3. **Verify, don't guess.** Open a definition before using it. Confirm the cause in the code, even when someone names a location.
4. **Keep fixes narrow and tested.** Fix what the issue describes, where the bug is. Add a test that fails without the fix.
5. **Review the diff before opening a pull request.** Identifiers exist as used, the test fails without the fix, nothing unrelated changed, and the description is true.
6. **Look for a better pull request.** If another fixes the same thing, close yours when it is better or equal, and say why.
7. **Report only what ran.** Say what you tested and what you could not run. Without a local build or test, say so and rely on CI before marking ready.
8. **`main` changes through a pull request.** Do not push to it. Branch protection enforces this. There is no exception.
9. **Stop and ask only about** licensing outside the allowlist, publishing, releases, a new remote, secrets, spending money, destructive git, and anything else hard to undo. Otherwise keep working. `plan/` is a gitignored notebook, not rules. Without it, do not ask for it and do not stop.
10. **Keep secrets and personal details out** of the repository, logs and requests.
11. **CI stays shared.** Workflows call `storytold/craft-common-actions`. Do not change CI in a feature or fix pull request. If CI cannot cover a change, open an issue there.
12. **Guards say why.** A check that looks removable says what it prevents. Read that before changing it.
13. **Instructions must be readable by everyone.** No notes about one machine, and no links to files a reader cannot open. `craftrules` is retired. Do not link to it.
14. **Keep pull requests moving.** Concurrent pull requests are fine; unanswered actionable feedback is not. Respond in 7 days, a reminder at 14, closure eligible at 30, from when an actionable request waits on the author. Suggestions and unconfirmed bot findings do not start the clock. A fix, explanation or agreed plan resets it; unrelated activity does not. Waiting on a reviewer or maintainer pauses it and does not count against the author. A maintainer decides closure; it is never automatic. Reuse search results, do not poll, and stop at a rate limit.
15. **No agent attribution.** A pull request does not say an agent wrote it. A repository rule may not require that.
16. **Shared libraries first for Craft apps.** Before adding or changing reusable file-format, codec, protocol or UI code, check `storytold/craft-libs` and relevant open pull requests. Reuse its public APIs or extend the appropriate shared library instead of duplicating an implementation in an app. Keep app-specific policy and model mapping local; preserve behavior, security and resource limits, attribution, layering and regression coverage. Document API gaps and maintainer-approved exceptions rather than silently dropping behavior. This does not authorize dependencies on sibling application crates or override existing platform, licensing or dependency-source requirements.
<!-- craft-house-rules:end -->
<!-- craft-house-rules:start rust source=279b2c09ef2b2f0b4f5e616a47d39e29db0a1eb3 sha256=56ef2b873de68940cf903069bbd0927b119f5c9ec2717f7f627041d2ebe8e4f2 -->
## Rust Craft repositories

These apply to native Craft apps and to shared Rust libraries. Select them by repository type. A repository that merely contains some Rust does not take this block. Rules later in a repository's file may be stricter, but not weaker. The source is `standards/house-rules-rust.md` in [storytold/craft-contrib](https://github.com/storytold/craft-contrib).

1. **Never crash.** Non-test code does not panic and has no `unsafe`, except where the repository file names an exception. Return errors, treat input as hostile, and bound recursion and allocations. New production crate roots copy the `deny` attribute from an existing crate: `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::unimplemented`, `clippy::todo`, `clippy::unreachable`. Every crash fix comes with a test that panicked before the fix.
2. **Clean room.** Learn from observed behaviour and public specifications. Do not copy another product's code, assets, strings or artwork. Check a dependency's licence before adding it.
3. **Assets and fonts.** Every asset is original or suitably licensed, and is recorded in `ATTRIBUTION.md` in the same change. Font files live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), not in the app repository.
4. **Rust only for native Craft apps.** No handwritten JavaScript or TypeScript, and no Tauri, Electron or webview shell, in those apps. Shared tooling follows its documented technical scope and maintainer-approved exceptions. Craft PR Review's existing Tauri and TypeScript UI is one such exception. It does not relax this rule for native apps. Preserve any existing web build.
5. **Time goes through `chrono`.** Do not hand-roll calendar conversion. For dates beyond what `chrono` can represent, follow [standards/time.md](https://github.com/storytold/craft-contrib/blob/main/standards/time.md).
6. **Contributor credits.** Add or edit only your own person's entry in `contributors/people.toml`, and only after asking them. Never guess a name or hand-edit `contributors.json`.

## Licence allowlist

Dependencies and assets use SPDX identifiers. An expression is allowed when every identifier is in this list. A repository may name a shorter list. It may not add an identifier here by editing its own copy of these rules.

- `ISC`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `Apache-2.0`
- `Unicode-3.0`
- `OFL-1.1`
- `Ubuntu-font-1.0`
- `Bitstream-Vera`
- `CC0-1.0`
- `CC-BY-4.0`
- `CC-BY-SA-4.0`
- `Zlib`
- `BSL-1.0`
- `0BSD`
- `Unlicense`
- `IJG`
- `NCSA`
- `CDLA-Permissive-2.0`
- `Apache-2.0 WITH LLVM-exception`
- `LicenseRef-Public-Domain`

`MPL-2.0` is allowed for third-party dependencies used unmodified; vendoring, patching or copying its code needs a maintainer's approval. `MPL-2.0+` counts as `MPL-2.0`.

`LicenseRef-Storyteller-Trademark` is not in this list. It covers Storyteller brand marks only, where a repository already says so.
<!-- craft-house-rules:end -->

### How CraftCloud differs from the blocks above

The three blocks above are copied unchanged from the Craft repositories. Where CraftCloud does not (yet) follow
them, this says why. Nothing here weakens a rule for new work; it says what exists today.

- **Not a storytold repository.** CraftCloud lives at https://github.com/jonasbove/craftcloud, its only home: the
  code, issues, pull requests, releases and images are all there. It follows the Craft apps' house rules by choice.
  Its CI calls `storytold/craft-common-actions` (core rule 11) for the Nix check (`nix.yml`, pinned to a commit).
  `.github/workflows/ci.yml` and `packaging-lint.yml` still write the shared gate and lint out: the shared workflows
  use StepSecurity's actions, which need a paid subscription in a private repository, and this repository was
  private when that was decided. It is public since 2026-11-14, so they move to the shared workflows, each in a pull
  request of its own. For the same reason there is no `house-rules.yml` workflow yet.
- **`main` changes only through a pull request** (core rule 8), enforced by a GitHub ruleset. **Every pull
  request is merged by a maintainer's explicit confirmation; agents open pull requests and ask, they never merge on
  their own.** One topic per pull request, on a branch (`fix/…`, `docs/…`, `feature/…`, `chore/…`), titled with a
  gitmoji subject; its description says what changed, what ran (the `cargo xtask ci` summary line, any live tests)
  and what could not run. The agent then asks the maintainer to merge that pull request, by number and title, and
  merges only after a yes for it.
- **Commit trailers.** Commits made with an agent keep their `Co-Authored-By:` trailer. Pull request descriptions
  follow core rule 15.
- **`plan/`** (core rule 9) is `PLAN.md` here: a gitignored notebook, not rules.
- **No panics** (Rust rule 1): `unsafe_code` is forbidden workspace-wide, and the three app roots
  (`apps/*/src/main.rs`) carry the `deny` attributes. The library crates `crates/{core,native,ui-egui}` do not carry
  them yet; adding them is its own task. New crate roots copy the attributes from `apps/craftcloud/src/main.rs`.
- **Fonts and assets** (Rust rule 3): Inter, Archivo and Geist Mono (OFL-1.1) are vendored in
  `crates/ui-egui/assets/fonts/` with their licences, because the web frontend builds them in. Credits are in
  `NOTICE`; there is no `ATTRIBUTION.md`. A new asset gets its line in `NOTICE` in the same change.
- **Rust only** (Rust rule 4): everything CraftCloud adds is Rust (egui; Trunk for the web). The one planned
  exception is `craftcloud-shim.js`, which the `-apps` image is to inject into Craft web apps that lack the
  `craft-web-host` bridge (`apps.toml`, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)). It is not in the tree yet;
  adding it is a maintainer decision. The one Python left is the design build in `design/tools/` (FigCraft, not
  shipped; `cargo xtask design` later, roadmap M10 part 2); the operator scripts in `scripts/` need only sh and curl,
  and the packaging icons come from `cargo xtask icons`.
- **Time** (Rust rule 5): CraftCloud uses `jiff`, and `crates/ui-egui/src/web/` still has some hand-rolled calendar
  code. New date code goes through a date crate; moving the old code over is its own task.
- **Contributor credits** (Rust rule 6): there is no `contributors/` folder yet; see
  [Contributor credits](#contributor-credits-about-window) below.
- **Licences:** CraftCloud's own code is MIT OR Apache-2.0. The server image runs the upstream OxiCloud image
  unmodified (MIT, `upstream/oxicloud/LICENSE`) with Caddy (Apache-2.0) in front; both are in the allowlist.
  A dependency licence check (`deny.toml`, `cargo deny`) follows the allowlist above, and is stricter: `MPL-2.0`,
  `Unlicense` and `CC0-1.0` pass only for the crates `deny.toml` names (its `exceptions`), so a new crate under one of
  them needs its own entry there.

# CraftCloud — instructions for agents

CraftCloud is a self-hosted cloud for the Craft apps: the unmodified [OxiCloud](https://github.com/AtalayaLabs/OxiCloud)
server in a Docker image with Caddy in front, CraftCloud's own Rust/egui web frontend (WebAssembly, Trunk), a Rust
desktop sync client that is also the `craftcloud` command line, and a small Rust admin service. Its look follows the
[ArtCraft Launcher](https://github.com/storytold/craft-launcher), and the workspace is laid out like it:
`crates/{core,native,ui-egui}`, `apps/{craftcloud,craftcloud-web,craftcloud-admin}`, `xtask` and `packaging/`, plus
`Dockerfile`, `docker/`, `compose.yaml`, `flake.nix` and `nix/` (the NixOS module). [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
has the map.

## Start every session here
1. Read [`docs/ROADMAP.md`](docs/ROADMAP.md) (what's next) and [`docs/STATUS.md`](docs/STATUS.md) (what works and how
   it was checked). Unless the user gives you a task, pick work from the roadmap.
2. Read the doc for the area you touch: [ARCHITECTURE](docs/ARCHITECTURE.md), [CLI](docs/CLI.md),
   [CRAFT-INTEGRATION](docs/CRAFT-INTEGRATION.md), [OIDC](docs/OIDC.md), [FILES-ON-DEMAND](docs/FILES-ON-DEMAND.md),
   [UPSTREAM](docs/UPSTREAM.md), [RELEASE-NOTES](docs/RELEASE-NOTES.md), [RELEASING](docs/RELEASING.md), [BACKUP](docs/BACKUP.md), [TESTING](docs/TESTING.md), and `design/README.md` for screens.
   People contributing by hand start at [CONTRIBUTING.md](CONTRIBUTING.md).
3. `PLAN.md` and `docs/RESEARCH.md` are an optional gitignored notebook, and `CLAUDE.local.md` holds one machine's
   facts. Do not ask for them and do not stop when they are absent.

## Non-negotiables
- **The sync never loses a file.** It moves people's work between computers, and a wrong delete spreads to every
  device. So:
  - **Never delete on uncertainty.** A place that can't be read (no permission, a failed or partial listing, an
    unplugged drive, a missing sync folder) is *unknown*: it is held and reported, never taken as gone. A missing
    sync folder is never made again on its own.
  - **One rule for what the sync skips** (`core::names::ignored`, CraftCloud's own `.craftcloud/`, names the cloud
    can't take), applied to the local scan, the cloud listing and the saved state alike. A name the sync leaves
    alone is never downloaded, uploaded or taken as deleted.
  - **A delete never removes a file for good:** it goes to the server's own trash (OxiCloud: over REST; Nextcloud
    and ownCloud: their WebDAV `DELETE`, which the check on connect confirms lands in the trash), or to
    `.craftcloud/trash/` where the server has none. A folder goes as one item, so a restore brings it back whole.
  - **Never trash a cloud folder** unless a fresh listing of it says it is empty (or everything in it is unchanged).
    A folder whose listing failed is unknown.
  - **Writes are guarded:** create with `If-None-Match: *`, overwrite with `If-Match: <etag>`; a 412 is a change made
    elsewhere, never something to force. Before an overwrite the old bytes are kept: in the server's own versions
    where the check on connect confirms it keeps them (Nextcloud, ownCloud), otherwise in
    `.craftcloud/versions/<path>/<time>` (5 kept; OxiCloud). Where the server ignores a guard, the client never syncs
    both ways ([WEBDAV-STORAGE.md](docs/WEBDAV-STORAGE.md)). Changes made in two places become a conflict copy, never
    a silent overwrite. Mass deletes wait for the person.
  - **Every device moves past a change to the sync rules together.** A change to what the sync skips or how it
    decides can make an older client undo it. Add an "upgrade all devices together" entry to
    [docs/RELEASE-NOTES.md](docs/RELEASE-NOTES.md) in the same change.
  - Every sync fix comes with a test that fails without it (`crates/core` for decisions, `crates/native` for the
    engine, `crates/native/tests/live.rs` against a real server).
- **The upstream server runs unmodified.** The image starts from the pinned OxiCloud image (v0.10.0, the same as the
  `upstream/oxicloud` submodule; move only to a release that has been verified). Never patch or compile OxiCloud.
  OxiCloud's own apps (Music, Photos, the Markdown editor, viewers, WOPI, CalDAV/CardDAV) are off by default, and
  CraftCloud adds no apps other than the Craft apps.
- **Upstream gaps go in [docs/UPSTREAM.md](docs/UPSTREAM.md)** (Craft apps, craft-launcher, OxiCloud), with what was
  observed and the fix proposed. Do not file issues or open pull requests upstream without asking.
- **The repository stays neutral.** No personal hostnames, people's names, private URLs, tokens or machine notes in
  tracked files. The project address is https://github.com/jonasbove/craftcloud. Demo data is `cloud.example.org`,
  the user John Doe (`john`), and the devices "Laptop" and "Studio". Machine-specific notes go in the gitignored
  `CLAUDE.local.md`.
- **Branding:** CraftCloud has its own name and mark (the mackerel). Never use the ArtCraft logo, mark or name in
  CraftCloud's branding (product name, header, icon). The web header label is "CRAFT APP SUITE", as in the launcher.
- **The window always opens.** Nothing slow blocks the main thread before the first frame: the keychain and the tray
  answer over D-Bus on Linux and may hang, so they run on threads of their own (`apps/craftcloud/src/main.rs`,
  `tray.rs`). A missing tray, keychain or notification service degrades the client, never stops it.
- **The client never asks for or stores your account password** on CraftCloud's own server, Nextcloud or ownCloud:
  it signs in through the server's browser sign-in (OxiCloud's OAuth 2.0 device flow, Nextcloud's Login Flow v2), or
  with an app password you make on the server ([docs/WEBDAV-STORAGE.md](docs/WEBDAV-STORAGE.md) § 10). **The limit:**
  a plain WebDAV server (a NAS, `rclone serve webdav`, Apache `mod_dav`) may offer only its account password. The
  client asks there for "an app or WebDAV password" and can't tell which it was given; if it is the account
  password, the client keeps it in the OS keychain like an app password, and the sign-in screen is to say so (not yet:
  the command line's prompt still reads "not your account password"; roadmap Upcoming). Tokens and
  passwords live in the OS keychain. Treat any of them like the account's password.
- **Never break wasm.** `crates/core` and `crates/ui-egui` build for `wasm32-unknown-unknown`, and `cargo xtask ci`
  checks it. Desktop-only code goes in `crates/native` or `apps/craftcloud`.
- **The gate:** `cargo xtask ci` (fmt, clippy `-D warnings`, tests, wasm clippy) before every commit; run
  `cargo fmt --all` first. Commit only when its summary says `all 4 steps passed` **and names the tree you meant to
  check** (`… in <path>`).
  - Parallel agents and worktrees: a private `CARGO_TARGET_DIR` per worktree, and edit only the files you own.
  - The Docker image's build caches no Rust build output and shares none with your target dir: a green
    `cargo xtask ci` doesn't mean the image builds, and a built image says nothing about your tree.
  - On a shared machine keep builds polite: `CARGO_BUILD_JOBS` at about half the cores, `nice`, and `< /dev/null` for
    background runs (cargo can stop on a tty read).
- **Commits:** one subject line in plain English that starts with a [gitmoji](https://gitmoji.dev):
  `<gitmoji> Area: what changed for the person`, with the tester's task id when there is one (`(T-064)`).
  Examples: `🐛 Sync: …` (a bug fix), `✨ Web: …` (a feature), `📝 Docs: …`, `👷 CI: …`, `📦 Releases: …`,
  `🔒️ Security: …`, `⬆️ Deps: …`, `♻️ …` (a refactor), `✅ …` (tests), `🙈 …` (`.gitignore`). One gitmoji per
  commit, the one that fits its main change. A pull request's title follows the same form, since it becomes the
  merge commit's subject; contributors needn't know gitmoji, because the **Claude pull request check** adds one
  to a title that has none.

## Running and looking at the app
- **Desktop client:** `cargo run --release -p craftcloud` (window and tray); `craftcloud --hidden` starts in the tray.
  With a command (`craftcloud status`, `ls`, `put`, `sync`, `daemon`, `server users ls` …) it runs that and exits;
  every command takes `--json`. Reference: [docs/CLI.md](docs/CLI.md).
- **Control channel:** a running client listens on a loopback port and writes the port and a secret to
  `control.json` in its settings folder; `craftcloud status|sync|pause|resume|files` talk to it
  (`crates/native/src/control.rs`).
- **Web frontend:** `cargo xtask web` (Trunk, into `dist/web`) or `cd apps/craftcloud-web && trunk serve`. It needs a
  server for the API.
- **Screens without a server:** egui_kittest renders them.
  `cargo test -p craftcloud-ui-egui --features shots -- --ignored shots` renders the desktop screens, and
  `CRAFTCLOUD_SHOTS=/some/dir cargo test -p craftcloud-ui-egui --features shots -- --ignored web_shots` the web pages.
  For UI work, look at the PNGs and compare them with `design/`.
- **A throwaway server:** build the image (`docker build -t craftcloud:dev .`), then
  `KEEP=1 PROJECT=craftcloud-<name> scripts/smoke-test craftcloud:dev <free port>`. `PROJECT` lets several run side
  by side; `WEB_APPS=on` needs the `-apps` image (`--target with-apps`). Tear it down afterwards:
  `docker compose -p craftcloud-<name> down -v`.
- **Live sync tests** against that server (on a fresh server they create the owner first; CI's `live` job runs them
  on every pull request against an image built from the tree, as a throwaway owner with a random password made for
  the run and masked in the log):
  `CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:<port> CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD=… CRAFTCLOUD_HTTP_IDLE_SECS=5 cargo test -p craftcloud-native --test live -- --test-threads 1`.
- **The two-client smoke test** (any server, a test account; never in CI): the same variables, then
  `cargo xtask e2e-sync` (`--bin PATH` for another build, `--keep` to look afterwards). Two scratch clients sync a
  fresh folder pass by pass through every case in [docs/TESTING.md](docs/TESTING.md), and it ends with a PASS/FAIL
  table; every pass's `--json` stays in `target/e2e-sync/<run>/passes/`. With an admin account it stops when another
  client is signed in to it (a stray client syncing it makes the results meaningless); with others it warns.
- **Server facts to rely on** (OxiCloud v0.10.0; recorded answers in `crates/core/tests/fixtures/oxicloud-0.10.0/`):
  - First run: `GET /api/auth/status` says `initialized:false`; `POST /api/setup {username, email, password}` without
    auth creates the owner. Sign-in: `POST /api/auth/login`, renewal `/api/auth/refresh`. A Bearer token works on
    `/api/*` and `/webdav/*`.
  - Device flow: `POST /api/auth/device/authorize` gives a code and `<public URL>/device?code=…`; the person approves
    on **our** `/device` page (`/api/auth/device/verify`); the client polls `/api/auth/device/token`. Devices:
    `GET`/`DELETE /api/auth/device/devices`.
  - File bytes go over WebDAV, everything else over REST. WebDAV has no file id, so versions are keyed by path (a
    rename starts a new history unless the client carries the versions along).
  - ETags change on every write, and a `PUT`'s ETag equals `PROPFIND`'s. **Folder ETags are not reliable**
    (UPSTREAM.md C8), so every folder is listed every cycle (one level at a time, four at once; `Depth: infinity`
    answers 403). A `PUT` into a missing folder answers 409 (make the parents first).
- **Settings in the image:** `/app/storage/.craftcloud/settings.env` on the data volume is the one source of truth
  (`CRAFTCLOUD_WEB_APPS`, `CRAFTCLOUD_DISABLED_APPS`); the environment only seeds the first start. `craftcloud-admin`
  (127.0.0.1:8088, behind Caddy) checks admin rights through `/api/auth/me`, writes the file, and answers Caddy's
  `forward_auth` gate before every `/apps/*` file. On the slim image the web-apps switch says "Needs the -apps
  image".

## Roadmap and progress docs
The progress docs:
- [`docs/ROADMAP.md`](docs/ROADMAP.md): stage, the state of each area, milestones, current focus, upcoming work,
  progress log and revision history.
- [`docs/STATUS.md`](docs/STATUS.md): what works and how it was checked, and the known limits.
- [`docs/RELEASE-NOTES.md`](docs/RELEASE-NOTES.md): what a person updating must know, above all "upgrade all devices
  together" changes.
- [`docs/UPSTREAM.md`](docs/UPSTREAM.md): gaps in OxiCloud, the Craft apps and craft-launcher, and the fixes proposed.
- Design: `design/README.md` and the passes in `docs/design-notes/`.

When a change lands, update in the same change: the area row, the progress log and the status line in
`docs/ROADMAP.md`; `docs/STATUS.md` if what works changed; and RELEASE-NOTES if updating needs care. Keep the
README's Status section in step.

## Contributor credits (About window)
- The desktop client's **Settings › About** shows the version, the platform, the licence, links to the source,
  release notes and issues, and the look credit (`crates/ui-egui/src/desktop/settings.rs`, `about_panel`). The web
  footer (`crates/ui-egui/src/web.rs`) carries the same links and look credit. The
  README's "License and credits" and `NOTICE` credit the work CraftCloud includes or adapts.
- Like craft-launcher, CraftCloud lists no per-person contributors in the app. If it adopts the Craft apps'
  `contributors/` credits (`contributors.json` from commit stats, `people.toml` with the names people chose for
  themselves, compiled into the About window), it follows VectorCraft's `docs/contributors.md`, and Rust rule 6
  applies: add only your own person's entry, after asking them.
- Never add, guess or copy anyone's name from git config, commit authors or GitHub profiles.

## CI workflows
- `.github/workflows/ci.yml` (**CI**): `test` runs `cargo xtask ci` on Linux and checks that `Cargo.lock` is
  unchanged; `web` builds the web frontend with Trunk; `live` builds the server image from the tree, starts it with
  `scripts/smoke-test` (`KEEP=1`) and runs `apps/craftcloud/tests/cli_live.rs` and `crates/native/tests/live.rs`
  against it, the server's logs kept as an artifact when they fail. Linux only. macOS and Windows build on `v*` tags
  and by hand (`release.yml`): a choice made while the repository was private, where their minutes cost 10× and 2×.
- `.github/workflows/release.yml` (**Release**, replaces the old `desktop.yml`): on `v*` tags and by hand, the
  Craft apps' release file set from `packaging/` (Linux AppImage, deb, rpm, tar.gz, Flatpak and Arch packages for
  x86_64 and aarch64; Windows x64 and arm64 MSI and portable zip; a universal macOS DMG; FreeBSD and Linux riscv64
  as untried extras that may fail). On a tag, a **draft** release (`CraftCloud v<ver>`, notes in the Craft apps' layout: `## What's
  Changed` from GitHub's generated notes, `**Full Changelog**`) with `SHA256SUMS.txt`, build-provenance attestations (public repository) and, when the `RELEASE_SIGNING_KEY`
  secret (in the `release` environment, which only `v*` tags may use) is set, a GPG `.asc` for every file. Its own workflow rather than craft-common-actions' (UPSTREAM.md D1).
  Builds are not code-signed in v1; the README says how to get past Gatekeeper and SmartScreen. The release job
  doesn't wait for FreeBSD and riscv64: a `late` job adds them to the draft when they finish. It does wait for
  `package-test`: every Linux package installed in clean distro containers (Debian 12, Ubuntu 22.04/24.04, Fedora
  41/42 on x86_64 and aarch64; Arch; the AppImage and tarball on Ubuntu 22.04), checked, signed in and synced against
  a server image built in the same run (`server-image`), upgraded from the previous release and removed
  (`packaging/linux/test-package.sh`, which also runs locally with Docker). Run by hand, it tests without a draft.
- `.github/workflows/image.yml` (**Server image**): on `v*` tags, the slim and `-apps` images for amd64 and arm64
  to `ghcr.io/<owner>/craftcloud` (`:<ver>`, `:latest`, `:<ver>-apps`, `:latest-apps`) with a provenance
  attestation, copied unchanged to Docker Hub when `DOCKERHUB_USERNAME` (variable) and `DOCKERHUB_TOKEN`
  (secret) are set; by hand it builds both without pushing.
- `.github/workflows/dockerhub-copy.yml` (**Copy images to Docker Hub**): by hand, copies a released version
  from ghcr.io to Docker Hub unchanged (same digests, so the build's attestation covers the copy).
- `.github/workflows/dockerhub-remove.yml` (**Remove a version from Docker Hub**): by hand, deletes a version's
  two tags there (never `latest`).
- `.github/workflows/dockerhub-page.yml` (**Docker Hub page**): the Docker Hub description and overview, from
  `docker/DOCKERHUB.md`, on a push that changes it and by hand.
- `.github/workflows/nix.yml` (**Nix**): on a change to the flake, `nix/`, `Cargo.lock` or itself, calls
  craft-common-actions' `nix.yml` (`nix flake check --no-build --all-systems`: the packages, the dev shell and the
  NixOS module evaluate on every system, nothing is built), pinned to a commit rather than `@main`: CraftCloud is
  not a storytold repository, so a breaking change there would come without a pull request here.
- `.github/workflows/packaging-lint.yml` (**Packaging lint**): actionlint, shellcheck on `packaging/`, and the
  desktop entry, AppStream file, Info.plist and MSI sources; shellcheck and the tests for `.github/scripts/`.
- `.github/workflows/claude-pr.yml` (**Claude pull request check**): on every pull request that isn't a draft or a
  bot's (`pull_request_target`), Claude adds a gitmoji to a title without an emoji, adds type, platform and area
  labels, and writes one automated review comment, updated on every push (does it do what it says, risky or
  malicious changes, the house rules, must fix and could be better). A maintainer still decides on every merge.
  The pull request's code is never checked out or run: Claude reads the diff from the API, can only read, and
  answers in JSON; a separate job without Claude's token applies only a gitmoji from `.github/gitmojis.json` and
  labels from `.github/labels.json` (`.github/scripts/claude-bot.sh`). Claude's file tools are confined to the
  workspace (`blockReadsOutsideWorkingDirectories` plus deny rules for `/proc`, `~/.claude`, `.git` and the like),
  and an answer that looks like it holds a credential is never posted: the review becomes a fixed notice and the
  job fails. Claude's answers reach the apply job as a scanned one-day artifact, never through env or outputs,
  which GitHub prints in the public log.
- `.github/workflows/claude-issues.yml` (**Claude issue labels**): on a new, edited or reopened issue, Claude adds
  type, platform and area labels, and `❓ needs info` when a bug report lacks steps, version or system. It posts no
  comment; a possible duplicate goes in the run's summary, labelled `👯 duplicate` only when it names an open
  issue it is identical to.
- Both need the `CLAUDE_CODE_OAUTH_TOKEN` secret (from `claude setup-token`) and skip with a notice without it.
  `.github/labels.json` is the one list of the repository's labels, with their colours, descriptions and group;
  the workflows never create labels, add only type, platform and area labels (and those two on issues), never
  remove one, and never re-add one a person removed.
- One app id everywhere: `io.github.jonasbove.craftcloud` (window app id, desktop entry, icons, AppStream, Flatpak,
  macOS bundle, start-at-login entries). `cargo xtask icons` regenerates every packaging icon from the logo.
- Don't add or change CI in a feature or fix change. Moving the workflows to `storytold/craft-common-actions` is its
  own task.

New workspace crates include `[lints] workspace = true` in `Cargo.toml`. Production crate roots declare the crate-level lint attributes before modules; crate-level `//!` documentation may precede them.
