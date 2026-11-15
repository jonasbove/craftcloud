# Testing CraftCloud

The layers, from the fastest to the slowest:

| What | Runs | Needs |
|---|---|---|
| `cargo xtask ci` | fmt check, clippy (`-D warnings`), the workspace tests, the wasm build: the gate before every commit, and what CI runs | nothing |
| `crates/native/tests/live.rs` | the sync engine against a real server: two computers in one process, renames, folder moves, deletes, quota, resumed downloads, a silent connection | a throwaway server |
| `apps/craftcloud/tests/cli_live.rs` | every command of the command line against a real server | a throwaway server |
| `cargo xtask e2e-sync` | two separate clients, the built `craftcloud` program, syncing through a server pass by pass: the smoke test before a release or after a sync change | any server, and an account to test with |

How to start a throwaway server and run the live tests is in [CONTRIBUTING.md](../CONTRIBUTING.md). This page is
about the last one.

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
