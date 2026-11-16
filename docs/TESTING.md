# Testing CraftCloud

The layers, from the fastest to the slowest:

| What | Runs | Needs |
|---|---|---|
| `cargo xtask ci` | fmt check, clippy (`-D warnings`), the workspace tests, the wasm build: the gate before every commit, and what CI runs | nothing |
| `PROPTEST_CASES=50000 cargo test -p craftcloud-core properties` | the property tests of the sync decisions with more generated cases than the gate's 256 | nothing |
| `crates/native/tests/live.rs` | the sync engine against a real server: two computers in one process, renames, folder moves, deletes, quota, resumed downloads, a silent connection | a throwaway server |
| `apps/craftcloud/tests/cli_live.rs` | every command of the command line against a real server | a throwaway server |
| `cargo xtask e2e-sync` | two separate clients, the built `craftcloud` program, syncing through a server pass by pass: the smoke test before a release or after a sync change | any server, and an account to test with |

How to start a throwaway server and run the live tests is in [CONTRIBUTING.md](../CONTRIBUTING.md). This page is
about the last one, and about [measuring coverage](#coverage).

## The two-client sync smoke test (`cargo xtask e2e-sync`)

It does by hand what a person checking the sync between two computers would do, only the same way every time:
two scratch clients, "Laptop" and "Studio", each the `craftcloud` command line with a settings folder, a home and a
sync folder of its own, sign in to the server as two devices and sync a fresh folder, one `sync --once` pass at a
time. After every change it checks the other side, and where it matters the cloud.

```sh
CRAFTCLOUD_TEST_SERVER=https://cloud.example.org CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD=… \
  cargo xtask e2e-sync
```

Without `--bin` it builds this tree's `craftcloud` (debug) and tests that; `--bin PATH` tests another build, such as
an installed release (`--bin "$(command -v craftcloud)"`). The server can be a throwaway one (on a fresh server the
run makes the owner first, as the live tests do) or a real one: the run only touches a folder of its own, but it
signs two devices in to the account, so use an account kept for testing.

| Option | Meaning |
|---|---|
| `--bin PATH` | The `craftcloud` program to test (default: build this tree's) |
| `--keep` | Keep the cloud folder and the clients' folders afterwards, to look at them |
| `--settle SECS` | The wait before every pass, more than 2 (default 3) |
| `--dir PATH` | Where the run directory goes (default: `<target>/e2e-sync`) |
| `--allow-other-devices` | Go on, with a warning, when other clients are signed in to the account (admin accounts) |

### What it does

1. **No other client.** Another client syncing the same account takes part in the run, and its results then mean
   nothing (a stray old client once made a lost file this way). OxiCloud lists an account's signed-in clients only
   to admins (`/api/admin/sessions`; the device list, `/api/auth/device/devices`, only holds codes not collected
   yet). So with an admin test account the run lists the account's sessions and stops when another client is
   signed in (unless `--allow-other-devices`), and at the end fails when one signed in or was used meanwhile.
   With any other account the server can't say: the run warns loudly (WARN rows) and you make sure yourself.
   Other `craftcloud` processes on this computer also get a warning, since they may sync another account. Either
   way the last case compares the cloud with both clients, which catches what a stray client changed.
2. **Two clients.** Each gets its own settings folder (`CRAFTCLOUD_CONFIG_DIR`), home and sync folder, with
   `CRAFTCLOUD_KEYCHAIN=off`, so the tokens stay in a file in its settings folder and never touch the keychain.
   Each signs in through the device flow (`craftcloud login --no-browser`); the run approves its code through the
   API with the test account's password, as the person would on the `/device` page. The password comes from the
   environment and is never printed or passed to the clients.
3. **A fresh folder.** The run makes a top-level folder `e2e-sync-<seconds>-<pid>`, switches every other
   top-level folder off on both clients and leaves the files at the top of the cloud out, so nothing else on the
   account is synced, changed or deleted.
4. **The cases**, in order, each a change on one client, a pass on it and a pass on the other:

   | Case | Checks |
   |---|---|
   | create, edit, rename, move, both ways | the other side has the file, with the right content, and not the old name |
   | unicode names | `Crème brûlée é ü 日本.txt`, `Été/fichier.txt` and `🐟 makrel.txt` arrive by name and content |
   | a 20 MB file | the same SHA-256 on both sides |
   | 200 small files | all 200 arrive, each with its content |
   | delete a file, delete a folder | gone on the other side, and in the cloud's trash, checked through the REST trash API (`/api/trash/resources`); the folder as one item, not file by file |
   | a conflict | both sides edit one file before syncing: each side ends with the file and one conflict copy, holding both edits |
   | temporary-looking names (T-063, T-064) | `notes.tmp` stays on the Laptop, is listed as `temporary` under `not_synced`, and never reaches the cloud; a folder named `projekt.tmp/` syncs, and is still there after three more pass pairs |

   | the end state | the run's cloud folder, the Laptop and the Studio hold the same files with the same sizes |

5. **The whole run:** every pass ended up to date (exit 0), and no other client was active meanwhile (admin
   accounts only).
6. **Cleanup,** also after a failure: the cloud folder and CraftCloud's kept versions of its files go to the trash
   and then for good, both clients sign out (`craftcloud logout`, which ends the sign-in on the server), the
   password sign-in ends, and the clients' folders (with their tokens) are deleted. With `--keep` the folders stay; the sign-ins
   still end. A run stopped with Ctrl+C cleans up nothing: its `e2e-sync-…` folder stays in the cloud, and its
   two clients stay signed in (their tokens are in the run directory's `laptop/config` and `studio/config`;
   `CRAFTCLOUD_CONFIG_DIR=<that folder> CRAFTCLOUD_KEYCHAIN=off craftcloud logout` ends each).

**Every pass waits first.** A file is held for 2 s after it changes (T-060), so a pass that starts sooner skips it
and the case would fail for no reason. The run waits `--settle` seconds (3 by default, never 2 or less) before
every pass.

### What it leaves

The run directory, `<target>/e2e-sync/e2e-sync-<seconds>-<pid>/`, stays after every run:

- `passes/NN-laptop.json`, `NN-studio.json`: every pass's `sync --once --json`, numbered in order (with `.err`
  beside it when the pass wrote to stderr);
- `log/`: every other command's output, and the two sign-ins;
- `report.md`: the PASS/FAIL table.

The table is printed at the end too. The exit code is 0 when every check passed, 1 when one failed. A WARN (a
non-admin account, other `craftcloud` processes here, other clients allowed with `--allow-other-devices`, something
cleanup couldn't remove)
doesn't fail the run, but says what to look at.

### When a case fails

Read the passes around it: the case's detail names what was missing or wrong, and the pass on the side that made
the change says in `activity` what it sent (`up`, `moved`, `deleted`, `conflict`), and in `not_synced` what it left
out. Run again with `--keep` to look at both sync folders and the cloud folder as they ended. A case that fails
only now and then is worth an issue too: the cases are made to pass every time.

## Coverage

Coverage is measured by hand now and then, not in CI. It shows which code no test reaches; read it with the live
tests in mind, since most of the sync engine's red lines are what `crates/native/tests/live.rs` covers.

### How to run it

`cargo-llvm-cov` is a developer tool (MIT OR Apache-2.0), not a dependency of CraftCloud:

```sh
rustup component add llvm-tools-preview --toolchain 1.100.0   # the pinned toolchain (rust-toolchain.toml)
cargo install cargo-llvm-cov --locked

export CARGO_TARGET_DIR=$HOME/.cache/cc-target-coverage CARGO_BUILD_JOBS=4
nice -n 10 cargo llvm-cov --workspace --locked --no-report < /dev/null   # the same tests as `cargo xtask ci`
cargo llvm-cov report --summary-only                                      # per file
cargo llvm-cov report --html --output-dir "$CARGO_TARGET_DIR/cov-html"    # line by line: cov-html/html/index.html
cargo llvm-cov report --lcov --output-path "$CARGO_TARGET_DIR/lcov.info"  # for editors and scripts
```

To count the live tests too, set the `CRAFTCLOUD_TEST_*` variables of a throwaway server before the first
`cargo llvm-cov` (and `--test-threads 1`, as for the live tests). Without them they pass without running.

### The last run

2026-11-16, `main` at `01f71f8`, cargo-llvm-cov 0.9.1, Rust 1.100.0, on Linux. 462 tests passed, none failed. **Not
counted:** the live tests (`crates/native/tests/live.rs`, 20 tests, and `apps/craftcloud/tests/cli_live.rs`), which
passed without running for want of a server, the `shots` screenshot tests (`--features shots`, ignored) and one
ignored font test. `cargo xtask e2e-sync` runs the built program, outside coverage.

| Crate | Lines | Functions |
|---|---|---|
| `crates/core` | 96.1% (2826 of 2942) | 95.9% |
| `crates/ui-egui` | 88.9% (9413 of 10587) | 89.6% |
| `crates/native` | 55.1% (2074 of 3762) | 46.8% |
| `apps/craftcloud-cli` | 65.2% (73 of 112) | 60.9% |
| `apps/craftcloud-admin` | 56.2% (145 of 258) | 56.2% |
| `apps/craftcloud` | 32.9% (1064 of 3230) | 34.3% |
| `xtask` | 34.1% (606 of 1779) | 23.5% |
| **Workspace** | **71.5%** (16201 of 22673); regions 72.4% | **67.9%** |

What the numbers mean:

- **`crates/core` is well covered**: `reconcile.rs` 94%, `names.rs` 99%, `sync.rs` 93%, the API modules 90–100%.
  The few lines left in `reconcile` are rare branches (a vanished folder with no synced files, a folder whose
  subfolders are all going); property tests for it are on their way.
- **`crates/native/src/sync.rs` is 45%**, because the engine itself (`Engine::try_cycle`, `apply`, `fetch`,
  `move_folders`, `hold_no_room`, `restore_trashed`) talks to a server, and no unit test builds an `Engine`.
  Its pure helpers (`scan_local`, `screen_names`, `split_deletes`, `bring_back`, the part files) are covered. The
  live tests cover the engine's main paths (two computers, renames, folder moves, deletes to the trash, quota,
  resumed downloads, held and discarded mass deletes); the table below lists what neither reaches.
- **`session.rs` is 13%**: the token refresh and the versions kept before an overwrite.
- **`apps/craftcloud`** is mostly the window, the tray and the commands' calls to the server: `main.rs`, `tray.rs`
  and `cli/files.rs` are glue that the CLI end-to-end test and people check, not unit tests. `xtask`'s `e2e`
  harness is itself a test.
- **The UI** is covered by the kittest tests at 80–100% per file; the largest unrendered parts are the admin's
  person details (`web/admin.rs`, `details_ui`), the file browser's grid view (`web/files/table.rs`, `grid_ui`,
  `tile_ui`) and the handling of some server answers (`web/files.rs`, `on_answer`; `web/admin.rs`,
  `on_admin_answer`).

### The gaps worth a test, by risk

"Live" means a live test covers the main path; the gap is the branch named. "Engine in a test" means a unit test
inside `sync.rs` that builds an `Engine` (no network needed); there is no such constructor yet. "Fake server"
means a local `TcpListener` thread answering scripted responses, as `http.rs`'s `stalled_server` test does.

| # | Where | What no test checks | Today | Test |
|---|---|---|---|---|
| 1 | `native::sync`, `Engine::replan` | Switching a folder off deletes the local files that are exactly as last synced; a file **changed since** must be kept and reported (`kept += 1`). Only the unchanged case is live-tested. A wrong size or time compare here deletes someone's work. | live, unchanged files only | Engine in a test |
| 2 | `native::sync`, `apply` → `trash_file`, `still_at`, `cloud_holds`; `RemoveRemoteFolder` | The "changed in the cloud meanwhile" guards: a file whose ETag moved since the plan is not trashed; a folder that holds more or newer files than planned is not trashed; a folder whose listing fails is kept (T-064). Live tests only take the happy path. | live, happy path only | fake server, or the comparisons pulled out into pure functions |
| 3 | `native::session`, `retrying`, `refresh` | A 401 refreshes and retries once; tokens another process saved are taken over, and when those are stale too a real refresh follows; a refused refresh (401, 403, 404) signs out instead of failing forever. No live test runs past the 1-hour token. | untested | fake server |
| 4 | `native::sync`, `Engine::resolve` | Settling a conflict with "keep the cloud's" deletes the conflict copy; "keep both" records it as the person's file. Only "keep mine" is live-tested. | live, `Keep::Mine` only | Engine in a test |
| 5 | `native::sync`, `Engine::cycle` | How a failed cycle becomes a state: signed out, offline (a transport error) or failed. The CLI's exit codes 3, 8 and 1 hang on it (`control::state_name` never produces `offline`, `signed_out` or `failed` in a test). | live for offline | Engine in a test with an unreachable server |
| 6 | `native::session`, `snapshot`; `apply`'s `UploadChange` | The old bytes are kept before an overwrite (not twice for one ETag, 5 kept), and a 412 on the overwrite (the cloud changed between the listing and the write) becomes a conflict copy. The live conflict test changes both sides before a pass, so the plan finds the conflict, not the 412. | live, versions kept | fake server |
| 7 | `craftcloud-admin`, `main.rs` (`handle`, `authorize`) | The route table: which routes ask for admin rights, `X-Forwarded-Uri` to the `/apps/*` gate, a settings `PUT` saved, unknown paths 404. `logic.rs` is tested, the wiring is 0%. Also `admin_check` failing closed (502) when OxiCloud's answer can't be read, and `bundled_apps` (an unreadable `apps.json` serves no apps). | untested (the smoke test checks some routes) | unit with `tiny_http` on port 0 and a fake OxiCloud; `admin_check` and `bundled_apps` plain unit |
| 8 | `native::secrets`, `TokenStore` | Saving, loading and deleting the tokens; with `CRAFTCLOUD_KEYCHAIN=off` the private file (what `e2e-sync` and headless servers use), and a working keychain removing an older file. 27%. | untested | unit with `CRAFTCLOUD_KEYCHAIN=off` (the variable is process-wide: one test, or a constructor switch) |
| 9 | `craftcloud` CLI, `output.rs`, `Fail::name`; `control::state_name` | The JSON error `kind` strings (`not_found`, `held` …) that scripts read never run in a test, nor exit code 6 (`no_client`). `account::failed` is tested with hand-written state strings, not with `state_name`'s, so a renamed state could make `sync` exit 0. | untested | unit, easy: compare with the table in [CLI.md](CLI.md) |
| 10 | `crates/ui-egui`, `web/admin.rs` `details_ui`, `on_admin_answer`; `web/files/table.rs` grid view | A person's details sheet in Admin › Users (role, quota, sessions) and how admin answers (errors included) land; the file browser's grid of tiles. | untested | kittest |

Below these, in rough order: `hold_no_room`'s recount for admins (live covers the quota itself),
`native::login` (start at login, per platform), `cli::headless::daemon`, and the font lookup in `native::lib`
(depends on the machine).
