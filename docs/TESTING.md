# Testing CraftCloud

The layers, from the fastest to the slowest:

| What | Runs | Needs |
|---|---|---|
| `cargo xtask ci` | fmt check, clippy (`-D warnings`), the workspace tests, the wasm build: the gate before every commit, and what CI runs | nothing |
| `PROPTEST_CASES=50000 cargo test -p craftcloud-core properties` | the property tests of the sync decisions with more generated cases than the gate's 256 | nothing |
| `cargo test --workspace` on Windows | the same workspace tests on Windows x64 (`windows-test` in `release.yml`, on `v*` tags and by hand; not on every pull request, since CI runs on Linux), so the Windows code (`core::names`' Windows rules, `craftcloud.com`, the window geometry, the Windows paths) is tested on Windows itself; the draft release waits for it (about 10 minutes) | nothing |
| `crates/native/tests/live.rs` | the sync engine against a real server: two computers in one process, renames, folder moves, deletes, quota, resumed downloads, a silent connection; each test syncs only folders of its own and removes them afterwards; CI's `live` job runs it on every pull request and push to `main`, against a server image built from the tree in the same job | a throwaway server (CI starts one) |
| `apps/craftcloud/tests/cli_live.rs` | every command of the command line against a real server; in CI's `live` job too, before `live.rs` | a throwaway server (CI starts one) |
| `cargo test -p craftcloud-native probe_a_real_server -- --ignored --nocapture` | the check on connect ([WEBDAV-STORAGE.md § 7.5](WEBDAV-STORAGE.md)) against a real WebDAV server, by hand: `CRAFTCLOUD_PROBE_URL` (the WebDAV base, e.g. `http://127.0.0.1:8095/remote.php/dav/files/john/`), `CRAFTCLOUD_TEST_USER`, `CRAFTCLOUD_TEST_PASSWORD` (Basic auth); prints every step and the mode. The scripted servers it is tested against in the gate are `core::storage::probe::fake` (well behaved, and each misbehaviour of § 5 as a switch) | a WebDAV server to try (never a real account's data: it writes in `.craftcloud/probe/` only) |
| `cargo xtask e2e-sync` | two separate clients, the built `craftcloud` program, syncing through a server pass by pass: the smoke test before a release or after a sync change | any server, and an account to test with |
| `cargo xtask e2e-web` | the web frontend the server serves, in headless Chromium: setup, sign-out and sign-in, an upload, a share link opened signed out, a device approval, the admin CSRF check | a fresh throwaway server, chromedriver and Chromium |
| The Linux desktops, by hand | the desktop client under KDE Plasma, GNOME and Xfce in headless containers: the tray's registration, notifications, the keychain, `craftcloud://`, the autostart entry and the window; what ran, the results, and what needs a real login are in [LINUX-DESKTOPS.md](LINUX-DESKTOPS.md) | Docker, a throwaway server |

How to start a throwaway server and run the live tests is in [CONTRIBUTING.md](../CONTRIBUTING.md). This page is
about the last two, and about [measuring coverage](#coverage).

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
| `--bin PATH` | The `craftcloud` program both clients run (default: build this tree's) |
| `--bin-laptop PATH`, `--bin-studio PATH` | The program one client runs, instead of `--bin` (or of this tree's build): a mixed pair, below |
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
   | a conflict, each side syncing first | both sides edit one file before syncing: each side ends with the file and one conflict copy, holding both edits |
   | delete a folder from the Studio | as from the Laptop: gone on the Laptop, and in the trash as one item |
   | versions, both ways | one client overwrites a synced file, so its sync keeps the old content in `.craftcloud/versions/`; the other lists it (`versions PATH`) and restores it: both sides end with the old content and no conflict copy, and the writer lists the overwritten content as a version too |
   | names Windows can't have, both ways | `a:b.txt`, `what?.md`, `CON.txt`, `Nul.log`, `trailing.`, a folder `dots./`, and `Report.txt` beside `report.txt` arrive on the other side, which edits one; after two more pass pairs from each side all are still on both sides and in the cloud (a Windows client leaves these alone, which must never read as deleted) |
   | leftovers | each client leaves part files (the `.<name>.<version>.craftcloud-part` and `.ccpart-<hex>.craftcloud-part` forms), a `.craft_tmp` file and a `.craftcloud/` folder in the run's folder: none reaches the other side or the cloud (listed with `--all`), each stays where it was left, and the cloud's `.craftcloud/` holds no client's sync state |
   | temporary-looking names (T-063, T-064) | `notes.tmp` stays on the Laptop, is listed as `temporary` under `not_synced`, and never reaches the cloud; a folder named `projekt.tmp/` syncs, and is still there after three more pass pairs |
   | the end state | the run's cloud folder, the Laptop and the Studio hold the same files with the same sizes |

5. **The whole run:** every pass ended up to date (exit 0), and no other client was active meanwhile (admin
   accounts only).
6. **Cleanup,** also after a failure: the cloud folder and CraftCloud's kept versions of its files go to the trash
   and then for good, both clients sign out (`craftcloud logout`, which ends the sign-in on the server), the
   password sign-in ends, and the clients' folders (with their tokens) are deleted. With `--keep` the folders stay; the sign-ins
   still end. With an admin account the run checks that the clients' logout really ended their sessions on the
   server (FAIL otherwise), and ends any it left itself, so the next run doesn't find them signed in. With 0.1.1 and
   this tree that check fails: their `logout` leaves the session signed in on the server, a known failure until the
   command line ends it. A run stopped with Ctrl+C cleans up nothing: its `e2e-sync-…` folder stays in the cloud, and its
   two clients stay signed in (their tokens are in the run directory's `laptop/config` and `studio/config`;
   `CRAFTCLOUD_CONFIG_DIR=<that folder> CRAFTCLOUD_KEYCHAIN=off craftcloud logout` ends each).

**Every pass waits first.** A file is held for 2 s after it changes (T-060), so a pass that starts sooner skips it
and the case would fail for no reason. The run waits `--settle` seconds (3 by default, never 2 or less) before
every pass.

### Mixed versions: an older release on one side

People don't update every computer at once, so a change to what the sync skips or how it decides can make an
older client undo it (the "upgrade all devices together" entries in [RELEASE-NOTES.md](RELEASE-NOTES.md)). Run the
same cases with the previous release on one side and this tree's build on the other, both ways round:

```sh
# The previous release's Linux tarball, checked against the release's SHA256SUMS.txt before it runs
V=0.1.1   # the previous release
gh release download v$V -R jonasbove/craftcloud -p "craftcloud-$V-linux-x86_64.tar.gz" -p SHA256SUMS.txt
sha256sum -c --ignore-missing SHA256SUMS.txt && tar -xzf craftcloud-$V-linux-x86_64.tar.gz
OLD=$PWD/craftcloud-$V-linux-x86_64/bin/craftcloud
cargo build -p craftcloud && NEW=$PWD/target/debug/craftcloud
cargo xtask e2e-sync --bin-laptop "$OLD" --bin-studio "$NEW"
cargo xtask e2e-sync --bin-laptop "$NEW" --bin-studio "$OLD"
```

The table's first row says which program each client ran. The versions, conflict, folder-delete, Windows-names and
leftovers cases are the ones a mixed pair is most likely to fail; a case that fails with a mixed pair but passes
with the same build on both sides is a finding for the release notes. A run on Linux or macOS can't show what a
Windows client leaves alone: the Windows-only name rule is checked on Windows. The run is made for Linux: on a
case-insensitive file system (macOS by default) `Report.txt` and `report.txt` can't both exist, so that case fails
there for a reason that isn't the sync's.

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

## The web frontend in a browser (`cargo xtask e2e-web`)

The web pages' own tests render them off-screen (egui_kittest) against a scripted backend, and `scripts/smoke-test`
checks the server's routes with curl. Neither runs the WebAssembly app in a browser against a real server, so
neither sees the session cookies, the CSRF header on writes, Caddy's routes or the `/device` page as a person does.
`e2e-web` does: headless Chromium, driven through chromedriver, opens the app the server serves and goes through
the cases below on a fresh server.

```sh
docker build -t craftcloud:dev .
KEEP=1 PROJECT=craftcloud-e2eweb scripts/smoke-test craftcloud:dev 8100   # a fresh server, no owner yet
cargo xtask e2e-web --server http://127.0.0.1:8100
docker compose -p craftcloud-e2eweb down -v
```

| Option | Meaning |
|---|---|
| `--server URL` | The server (default: `CRAFTCLOUD_TEST_SERVER`). It must have no owner yet: the first case is first-run setup |
| `--shots` | Save a screenshot after every step, in `<run>/shots/` (a failed case always saves one) |
| `--dir PATH` | Where the run directory goes (default: `<target>/e2e-web`) |

The owner it makes is `CRAFTCLOUD_TEST_USER` (default `john`) with `CRAFTCLOUD_TEST_PASSWORD`, or a new random
password that is never printed. It never runs in CI.

### What it needs

- **chromedriver and Chromium (or Chrome) of the same major version**, on `PATH` (`chromium`,
  `google-chrome-stable` or `google-chrome`), or named by `CRAFTCLOUD_E2E_CHROMEDRIVER` and `CRAFTCLOUD_E2E_CHROME`.
  Distributions package them together (`chromium` with its `chromedriver` on Arch, `chromium-driver` on Debian);
  for Google Chrome, chromedriver comes from the [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)
  downloads. Without them the run stops at once and says which one is missing.
- No GPU: the browser draws with its software renderer (SwiftShader), as on a server.
- Firefox and geckodriver have not been tried; the network-log checks below use Chromium's log.
- No WebDriver crate: `xtask/src/e2e_web/webdriver.rs` speaks the few W3C WebDriver commands the run needs over
  `ureq`, with a 60 s deadline on each, so a hung driver stops the run with an error. (The crates available bring
  in dependencies outside the licence allowlist.)

### How it drives a canvas

The app draws everything on one `<canvas>`, so the page has no buttons or fields for a WebDriver selector to find.
The options, and why the run uses the keyboard:

- **The accessibility tree** would be the most robust handle (widgets by role and name), but eframe doesn't build it
  on the web yet: in the pinned eframe, `crates/eframe/src/web/app_runner.rs` drops the AccessKit update ("not
  currently implemented"), and its `inspection` feature, which serves the tree to a test tool, does nothing on
  wasm.
- **Clicking at positions** ties every step to pixel layout: the sheets change layout at 600 and 640 px, and any
  change of size or spacing moves every button.
- **The keyboard** (chosen): egui moves the focus with Tab and Shift+Tab in the order it draws the widgets, Enter
  presses the focused one, and typing goes to the focused field through eframe's text input. That order changes
  only when a page gains or loses a widget, and it is how a person without a mouse uses the app. With nothing
  focused, Shift+Tab goes to the last widget, and a sheet draws its main button last, so most steps are
  "Escape, Shift+Tab, Enter". The one click the run makes is on an empty spot of the header, to give the page the
  keyboard (or to let go of the focus where Escape would close a sheet).

Every step is then checked where it shows, never in pixels: the server's API (with a password sign-in of the run's
own, independent of CraftCloud's client code), the browser's cookie jar (HttpOnly cookies included), and
Chromium's network log (which requests the page sent, with which headers; values are compared, never printed).
The window is 1280×800 and the Tab counts in `xtask/src/e2e_web/cases.rs` hold for that width; each says which
widgets it passes.

### The cases

| Case | Steps | Checks |
|---|---|---|
| first-run setup | the five setup pages: START SETUP, the owner's four fields and CREATE ACCOUNT, CONTINUE, CONTINUE (web apps; the slim image), GO TO HOME | the server has its owner; the browser holds a session cookie and `/api/auth/me` with its cookies names the owner |
| sign out | SIGN OUT in the header | the page sent `POST /api/auth/logout`; the session cookie is gone; `/api/auth/me` answers 401 |
| sign in, session cookie | the sign-in page: username, password, Enter | signed in as the owner; `oxicloud_access` is HttpOnly, Secure exactly when the address is https, and `oxicloud_csrf` is readable by the page |
| upload | UPLOAD FILES on the empty My files page, the file handed to the page's file input (`#craftcloud-upload-input`) | the file reads back the same over WebDAV; its `PUT` carried `X-CSRF-Token` equal to the `oxicloud_csrf` cookie |
| share link, opened signed out | the file's "…" menu, SHARE LINK…, CREATE LINK; then `/s/<token>` in a second browser with its own profile, and its DOWNLOAD | a new link on the server for the file; the second browser has no session, the page's `GET /api/s/<token>` answered 200, and the download holds the same bytes |
| device approval | the device flow started over the API (as a desktop client would); `/device?code=…` in the signed-in browser, APPROVE | the approval's `POST` carried the CSRF header; polling `/api/auth/device/token` gives a token that signs in as the owner |
| admin change | Admin › Users, NEW USER, its fields, CREATE | the user exists; the `POST /api/admin/users` carried `X-CSRF-Token` equal to the cookie |
| forged admin writes | the browser's cookies replayed outside it, as a forged cross-site request would arrive | `PUT /craftcloud/api/settings` without the header and with a wrong one: 403; with the right one: 200 (the control); `POST /api/admin/users` without it: 403, and no user made |

Sign-ins are limited per address (OxiCloud answers "too many sign-in attempts" after a handful in a minute), so
the run signs in three times in all: setup's, the sign-in case's and its own.

### What it leaves, and what it doesn't cover

The run directory, `<target>/e2e-web/<seconds>/`, keeps `report.md` (the PASS/FAIL table, also printed),
`chromedriver.log`, the screenshots, and each browser's profile and downloads. The exit code is 0 when every case
passed. The server keeps the owner, the uploaded file, the share link and the new user: use a throwaway server.

Not covered: how the pages look (the egui_kittest screenshots are for that), Firefox and Safari, phone widths,
single sign-on (tested by hand, [OIDC.md](OIDC.md)), the `-apps` image's web-apps switch (the slim image can't
change it, so its settings write is checked only from outside the browser), and pasting or drag and drop.
egui moves the focus once per frame, so the run waits 300 ms after every key (two Tabs in one frame count as
one; on a busy machine software rendering makes frames slow). Three steps are timing-sensitive: a Tab that arrives in the same frame as the last typed letters can be lost (the
run waits after typing); a Shift+Tab right after a page changes can start one widget off, since with nothing
focused it goes to the last widget of the frame before (setup's owner page); and the click that lets go of NEW
USER's focus under its sheet doesn't always take. The setup and admin cases then try once more from a fresh page
and say so in their detail. A case
that fails after a UI change usually means a Tab count moved: the failure's screenshot shows where the focus went.

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

A snapshot to measure again before a release, not a target: 2026-11-16, `main` at `01f71f8`, cargo-llvm-cov 0.9.1, Rust 1.100.0, on Linux. 462 tests passed, none failed. **Not
counted:** the live tests (`crates/native/tests/live.rs`, 20 tests, and `apps/craftcloud/tests/cli_live.rs`), which
passed without running for want of a server, the `shots` screenshot tests (`--features shots`, ignored) and one
ignored font test. `cargo xtask e2e-sync` runs the built program, outside coverage.

`crates/native` was measured again on 2026-11-16 after the engine, trash and sign-in tests
(`cargo llvm-cov -p craftcloud-native --summary-only`): 90 tests passed, 3 ignored (two known bugs, one font
test). Its row below is that run, and counts the test helper `fake_server.rs` (104 lines); the other rows are
the run above.

| Crate | Lines | Functions |
|---|---|---|
| `crates/core` | 96.1% (2826 of 2942) | 95.9% |
| `crates/ui-egui` | 88.9% (9413 of 10587) | 89.6% |
| `crates/native` | 67.1% (2757 of 4108) | 59.5% |
| `apps/craftcloud-cli` | 65.2% (73 of 112) | 60.9% |
| `apps/craftcloud-admin` | 56.2% (145 of 258) | 56.2% |
| `apps/craftcloud` | 32.9% (1064 of 3230) | 34.3% |
| `xtask` | 34.1% (606 of 1779) | 23.5% |
| **Workspace** | **71.5%** (16201 of 22673); regions 72.4% | **67.9%** |

What the numbers mean:

- **`crates/core` is well covered**: `reconcile.rs` 94%, `names.rs` 99%, `sync.rs` 93%, the API modules 90–100%.
  The few lines left in `reconcile` are rare branches (a vanished folder with no synced files, a folder whose
  subfolders are all going); property tests for it are on their way.
- **`crates/native/src/sync.rs` is 57%** (45% before). Most of the engine (`Engine::try_cycle`, `fetch`,
  `move_folders`, `hold_no_room`, `restore_trashed`) talks to a server; `sync/engine_tests.rs` builds an
  `Engine` against a scripted server (`crates/native/src/fake_server.rs`) for switching folders off, the guards
  before a trash and the sign-in during a cycle, and `sync/golden_tests.rs` runs whole cycles of 15 scenarios
  against a small scripted OxiCloud and compares every request, activity line and saved state with the logs in
  `sync/golden/` (`CRAFTCLOUD_BLESS=1` records them again, only for a change meant to alter what goes over the
  wire), checking on the way that every write carries its guard. Its pure helpers (`scan_local`, `screen_names`, `split_deletes`, `bring_back`, the part files) are covered. The
  live tests cover the engine's main paths (two computers, renames, folder moves, deletes to the trash, quota,
  resumed downloads, held and discarded mass deletes); the table below lists what neither reaches.
- **`session.rs` is 73%** (13% before): the token refresh, the take-over and the sign-out are tested against the
  scripted server; the versions kept before an overwrite are not (row 6).
- **`apps/craftcloud`** is mostly the window, the tray and the commands' calls to the server: `main.rs`, `tray.rs`
  and `cli/files.rs` are glue that the CLI end-to-end test and people check, not unit tests. `xtask`'s `e2e`
  harness is itself a test.
- **The UI** is covered by the kittest tests at 80–100% per file; the largest unrendered parts are the admin's
  person details (`web/admin.rs`, `details_ui`), the file browser's grid view (`web/files/table.rs`, `grid_ui`,
  `tile_ui`) and the handling of some server answers (`web/files.rs`, `on_answer`; `web/admin.rs`,
  `on_admin_answer`).

### The gaps worth a test, by risk

"Live" means a live test covers the main path; the gap is the branch named. "Engine in a test" means a unit test
in `sync/engine_tests.rs` that builds an `Engine` (no network needed). "Fake server" means
`crates/native/src/fake_server.rs`: a local `TcpListener` answering scripted responses, which the real `Session`
and `Engine` talk to.

| # | Where | What no test checks | Today | Test |
|---|---|---|---|---|
| 1 | `native::sync`, `Engine::replan` | Switching a folder off deletes the local files that are exactly as last synced; a file **changed since** (size, or only its time) is kept and reported (`kept += 1`). | **covered**: `switching_a_folder_off_keeps_the_files_changed_since_the_last_sync`, `switching_a_folder_off_without_changes_reports_nothing_kept` | Engine in a test |
| 2 | `native::sync`, `apply` → `trash_file`, `still_at`, `cloud_holds`; `RemoveRemoteFolder` | The "changed in the cloud meanwhile" guards: a file whose ETag moved since the plan is not trashed; a folder that holds more or newer files than planned is not trashed; a folder whose listing fails is kept (T-064); a file listed without an ETag can't be shown unchanged, so neither it nor its folder is trashed. | **covered**: `a_file_changed_in_the_cloud_since_the_plan_is_not_trashed`, `a_file_listed_without_an_etag_is_not_trashed`, `a_folder_with_a_file_listed_without_an_etag_is_not_trashed`, `a_file_whose_folder_cant_be_listed_is_not_trashed`, `an_unchanged_file_goes_to_the_trash`, `a_folder_with_a_file_added_in_the_cloud_is_not_trashed`, `a_folder_with_a_file_changed_in_the_cloud_is_not_trashed`, `a_folder_with_a_subfolder_that_cant_be_listed_is_not_trashed`, `a_folder_holding_exactly_what_was_planned_goes_to_the_trash_whole`, `a_folder_whose_listing_fails_is_kept`, `a_folder_that_is_not_empty_in_the_cloud_is_kept`, `an_empty_folder_goes_to_the_trash` | fake server |
| 3 | `native::session`, `retrying`, `refresh` | A 401 refreshes and retries once; tokens another process saved are taken over, and when those are stale too a real refresh follows with their refresh token; a refused refresh (401, 403, 404) signs out instead of failing forever; tokens stay out of errors and the log, at trace level too (the HTTP library stops at debug, `craftcloud_core::logging`). | **covered**: `an_expired_access_token_is_refreshed_once_and_the_request_sent_again`, `a_request_refused_again_after_a_refresh_is_not_retried_forever`, `a_refused_refresh_signs_out_instead_of_retrying`, `trouble_on_the_server_during_a_refresh_is_not_a_sign_out`, `tokens_another_process_saved_are_taken_over_without_a_refresh`, `stale_tokens_taken_over_are_refreshed_with_their_own_refresh_token`, `tokens_never_reach_an_error_or_the_log`, `tokens_never_reach_the_trace_log`, and the binary with `RUST_LOG=trace` in `apps/craftcloud/tests/cli_logout.rs` (`trace_logging_leaves_out_the_tokens`); in the engine `a_token_that_runs_out_mid_scan_is_refreshed_once`, `a_cycle_whose_refresh_is_refused_ends_signed_out` | fake server |
| 4 | `native::sync`, `Engine::resolve` | Settling a conflict with "keep the cloud's" deletes the conflict copy; "keep both" records it as the person's file. Only "keep mine" is live-tested. | live, `Keep::Mine` only | Engine in a test |
| 5 | `native::sync`, `Engine::cycle` | How a failed cycle becomes a state: signed out, offline (a transport error) or failed. The CLI's exit codes 3, 8 and 1 hang on it (`control::state_name` never produces `offline`, `signed_out` or `failed` in a test). | live for offline; signed out by `a_cycle_whose_refresh_is_refused_ends_signed_out` | Engine in a test with an unreachable server |
| 6 | `native::session`, `snapshot`; `apply`'s `UploadChange` | The old bytes are kept before an overwrite (not twice for one ETag, 5 kept), and a 412 on the overwrite (the cloud changed between the listing and the write) becomes a conflict copy. The live conflict test changes both sides before a pass, so the plan finds the conflict, not the 412. | live, versions kept | fake server |
| 7 | `craftcloud-admin`, `main.rs` (`handle`, `authorize`) | The route table: which routes ask for admin rights, `X-Forwarded-Uri` to the `/apps/*` gate, a settings `PUT` saved, unknown paths 404. `logic.rs` is tested, the wiring is 0%. Also `admin_check` failing closed (502) when OxiCloud's answer can't be read, and `bundled_apps` (an unreadable `apps.json` serves no apps). | untested (the smoke test checks some routes) | unit with `tiny_http` on port 0 and a fake OxiCloud; `admin_check` and `bundled_apps` plain unit |
| 8 | `native::secrets`, `TokenStore` | Saving, loading and deleting the tokens; with `CRAFTCLOUD_KEYCHAIN=off` the private file (what `e2e-sync` and headless servers use), and a working keychain removing an older file. 27%. | untested | unit with `CRAFTCLOUD_KEYCHAIN=off` (the variable is process-wide: one test, or a constructor switch) |
| 9 | `craftcloud` CLI, `output.rs`, `Fail::name`; `control::state_name` | The JSON error `kind` strings (`not_found`, `held` …) that scripts read never run in a test, nor exit code 6 (`no_client`). `account::failed` is tested with hand-written state strings, not with `state_name`'s, so a renamed state could make `sync` exit 0. | untested | unit, easy: compare with the table in [CLI.md](CLI.md) |
| 10 | `crates/ui-egui`, `web/admin.rs` `details_ui`, `on_admin_answer`; `web/files/table.rs` grid view | A person's details sheet in Admin › Users (role, quota, sessions) and how admin answers (errors included) land; the file browser's grid of tiles. | untested | kittest |

Below these, in rough order: `hold_no_room`'s recount for admins (live covers the quota itself),
`native::login` (start at login, per platform), `cli::headless::daemon`, and the font lookup in `native::lib`
(depends on the machine).
