# Contributing

Thanks for helping. The rules every change follows (people and AI agents alike) are in
[AGENTS.md](AGENTS.md); how the pieces fit together is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
In short:

- **One change per PR**, on a branch named `feature/`, `bugfix/`, `docs/`, `test/`, `refactor/` or
  `chore/` plus a short topic (`bugfix/sync-temp-folders`). The title is the commit message: a
  [gitmoji](https://gitmoji.dev), then plain English, what changed for the person or developer
  (`🐛 Sync: temp-looking folders sync again`).
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

The live tests create the owner on a fresh server and are skipped unless those variables are set
(never in CI). Use a throwaway server only, never one with real files.

Code is dual-licensed under MIT or Apache-2.0 ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)); by contributing you agree your work is licensed the same way.
