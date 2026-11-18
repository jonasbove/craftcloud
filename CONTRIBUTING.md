# Contributing

Thanks for helping. The rules every change follows (people and AI agents alike) are in
[AGENTS.md](AGENTS.md); how the pieces fit together is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
In short:

- **One change per PR**, on a branch named `feature/`, `bugfix/`, `docs/`, `test/`, `refactor/` or
  `chore/` plus a short topic (`bugfix/sync-temp-folders`). The title is the commit message: plain
  English, what changed for the person or developer (`Sync: temp-looking folders sync again`). The
  repository's commits start with a [gitmoji](https://gitmoji.dev) (`🐛 Sync: …`); you can add one,
  but you don't need to: a bot adds it (see below).
- **The server stays upstream OxiCloud, unmodified.** CraftCloud adds the desktop client, the web
  frontend and a small admin service around it. Something the server can't do goes into
  [docs/UPSTREAM.md](docs/UPSTREAM.md) as a gap, not into a patch.
- **Sync rules are a contract between computers.** A change to what the sync uploads, downloads,
  ignores or deletes comes with a test, and with an entry in
  [docs/RELEASE-NOTES.md](docs/RELEASE-NOTES.md) when older clients would act differently on the
  same account.
- **Neutral data only:** demo users are "John Doe" (`john`), servers are `cloud.example.org`. No
  real names, hosts, tokens or private files in code, tests, fixtures, screenshots or docs.
- **Everything is our own work:** no code, names or wording copied from other projects, and never
  the ArtCraft name or mark in CraftCloud's branding.

Before you ask for review:

```sh
cargo fmt --all
cargo xtask ci          # fmt check, clippy -D warnings, tests, wasm clippy; must end "all 4 steps passed"
cargo deny check        # licences, advisories, sources (deny.toml); CI runs it too
```

The Craft web builds in the `-apps` image are pinned in `apps.toml`. `cargo xtask bump-apps --check` says which
pins are behind the apps' latest releases and `cargo xtask bump-apps` updates them; `cargo xtask fetch-apps
apps.toml <out-dir>` builds the tree the image serves (the Dockerfile's `apps` stage runs it).

A new logo: replace `design/assets/logo/craftcloud-logo-1024.png` and run `cargo xtask icons`, which renders
the packaging icons from it (the Linux hicolor PNGs, `apps/craftcloud/assets/craftcloud.ico`,
`packaging/macos/craftcloud.icns`); commit what it writes.

Working in several checkouts at once (git worktrees): give each its own `CARGO_TARGET_DIR`, so two
builds never share or lock one target directory.

Changes that touch the server, the API client or the sync also run against a real server. Build
the image and start a throwaway stack with its own compose project, then point the live tests at it:

```sh
docker build -t craftcloud:dev .
KEEP=1 PROJECT=craftcloud-mine scripts/smoke-test craftcloud:dev 8093
CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8093 CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD=… \
  cargo test -p craftcloud-native --test live -- --test-threads 1
docker compose -p craftcloud-mine down -v
```

The live tests create the owner on a fresh server and are skipped unless those variables are set.
CI's `live` job runs them, and `cli_live`, on every pull request against an image built from it.
Use a throwaway server only, never one with real files. Each test syncs only top-level
folders of its own (`live-<test>-<seconds>-<pid>-…`) and, passed or failed, moves them and their kept versions to the
trash and deletes them from there, so what is already on the account doesn't change the results.

A sync change also gets the two-client smoke test, `cargo xtask e2e-sync` with the same variables: two scratch
clients sync a fresh folder pass by pass and check each other, and it ends with a PASS/FAIL table
([docs/TESTING.md](docs/TESTING.md)).

A change to the storage trait, the generic WebDAV driver or the check on connect also runs the live tests
against the other kinds of server, one at a time: `tests/other-servers/compose.yaml` starts Nextcloud, ownCloud,
OpenCloud, `rclone serve webdav` and Apache `mod_dav` (images pinned by digest, loopback only), and the same
`live` and `cli_live` suites run against each with `CRAFTCLOUD_TEST_KIND` and `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`.
[Its README](tests/other-servers/README.md) has the commands per server and what each is expected to get.

## What the bot does

Claude, run by the repository's GitHub Actions, looks at every pull request and issue:

- **Pull requests** (once they are out of draft): it puts a gitmoji in front of a title that has
  none, leaving the rest of the title as you wrote it; it adds labels for the kind of change, the
  platform and the parts it touches; and it posts one review comment, updated on every push: does
  the change do what the title and description say, does anything look risky, does it follow
  [AGENTS.md](AGENTS.md), and what must be fixed or could be better. The review is automated and
  can be wrong; it is not an approval. A maintainer reads every pull request and decides on
  merging.
- **Issues**: it adds labels (kind, platform, area, and `❓ needs info` when a bug report lacks the
  steps, the version or the system). It doesn't comment; the maintainer answers.

It only reads: it never runs your code, and it never removes a label or adds one a maintainer
took off. Bots' pull requests and issues are left alone.

Code is dual-licensed under MIT or Apache-2.0 ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)); by contributing you agree your work is licensed the same way.
