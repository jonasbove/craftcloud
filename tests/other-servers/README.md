# Throwaway servers for the live tests against other WebDAV servers

`compose.yaml` starts the servers CraftCloud's live tests run against besides its own
([docs/WEBDAV-STORAGE.md § 13](../../docs/WEBDAV-STORAGE.md), the Live row): Nextcloud 31, ownCloud 10.15,
OpenCloud 9.1.1, and two plain WebDAV servers, `rclone serve webdav` and Apache httpd with `mod_dav`. Every
image is pinned by digest; everything listens on the loopback address only; the passwords come from the
environment. They are test containers, never shipped and never linked: their licences (AGPL, Apache) don't
reach CraftCloud. Never put real data on them.

## Start

Set the five passwords (any throwaway values; all are required, even for a service you don't start) and the
ports if the defaults (8111 to 8115) are taken, then start one server at a time on a small machine: Nextcloud,
ownCloud and OpenCloud take a few hundred megabytes each.

```sh
export NC_PASS=… OC_PASS=… OPC_PASS=… DAV_PASS=…      # or put them in an env file: --env-file FILE
docker compose -f tests/other-servers/compose.yaml -p craftcloud-others up -d nextcloud
docker compose -p craftcloud-others logs -f nextcloud  # until it says it is ready (a minute or two the first time)
```

| Service | Port | User | What the sync is expected to get | Why |
|---|---|---|---|---|
| `nextcloud` | 8111 | `john` | two-way | every guard honoured; its own trash and versions, used through its profile (the suite connects with `--kind nextcloud`'s profile, and the command line's test signs in with `login --kind nextcloud --user john`, the app password from the environment, since the browser's Login Flow can't be driven headless) |
| `owncloud` | 8112 | `john` | two-way | the same |
| `opencloud` | 8113 | `admin` | download only | ignores `If-None-Match: *` on `PUT` (§ 16, Q1) |
| `rclone` | 8114 | `john` | download only | ignores `If-Match` and `If-None-Match` on `PUT` (golang/go#44492; § 16, Q2) |
| `apache` | 8115 | `john` | two-way, with CraftCloud's own trash and versions in `.craftcloud/` | keeps every guard (measured, below) |

OpenCloud's built-in admin is `admin`; the compose file turns its basic-auth switch on
(`PROXY_ENABLE_BASIC_AUTH`), which only an admin can do and the product never relies on (its own sign-in comes
with its profile, § 14 row 8). Nextcloud and ownCloud are set up with `john` as their admin; a plain user does
as well.

## App passwords

The client never signs in with an account password (§ 10). The tests make an app password the way a person
would and sign the client in with that:

- **Nextcloud**: `GET /ocs/v2.php/core/getapppassword` with the account's basic auth (the call Nextcloud's
  own clients make). The account password is used for that one call.
- **ownCloud**: Settings › Security › "App passwords / tokens", through the web session as a browser does
  (`curl` must be on `PATH`). The account password is used to sign in to the web once.
- **OpenCloud**: `POST /auth-app/tokens?expiry=72h` with the account's basic auth (the call behind its web
  UI's "App tokens"; on by default in 9.1.1) gives an app token, which is then the WebDAV password. The account
  password is used for that one call, which is what the basic-auth switch above is on for.
- **Plain WebDAV** (`rclone`, `apache`): the WebDAV password itself (§ 16, Q6).

To hand one in instead (made in the server's settings), set `CRAFTCLOUD_TEST_APP_PASSWORD`.

## Run the suites

Each run needs the opt-in (`CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`; without it a run naming another kind fails
at once, by design), the kind, the server's origin, the user and the password; a plain WebDAV server also
needs the mode it must get (`CRAFTCLOUD_TEST_MODE`), since that depends on the server. The suite asserts the
mode on every pass: a server that got better or worse is a failure, not a skip. The two-way tests run where
two-way sync is expected, the download-only tests (§ 9) where download only is.

```sh
export CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 CRAFTCLOUD_HTTP_IDLE_SECS=5

# Nextcloud (about 4 uploads a second: the 300-file test takes a while)
CRAFTCLOUD_TEST_KIND=nextcloud CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8111 \
CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD="$NC_PASS" \
  cargo test -p craftcloud-native --test live -- --test-threads 1

# ownCloud
CRAFTCLOUD_TEST_KIND=owncloud CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8112 \
CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD="$OC_PASS" \
  cargo test -p craftcloud-native --test live -- --test-threads 1

# OpenCloud (download only)
CRAFTCLOUD_TEST_KIND=opencloud CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8113 \
CRAFTCLOUD_TEST_USER=admin CRAFTCLOUD_TEST_PASSWORD="$OPC_PASS" \
  cargo test -p craftcloud-native --test live -- --test-threads 1

# rclone serve webdav (download only): the server address is the WebDAV address
CRAFTCLOUD_TEST_KIND=webdav CRAFTCLOUD_TEST_MODE=download_only CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8114 \
CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD="$DAV_PASS" \
  cargo test -p craftcloud-native --test live -- --test-threads 1

# Apache mod_dav (two-way, with the client's own trash and versions)
CRAFTCLOUD_TEST_KIND=webdav CRAFTCLOUD_TEST_MODE=two_way CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8115/dav \
CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD="$DAV_PASS" \
  cargo test -p craftcloud-native --test live -- --test-threads 1
```

The same variables run the command line's test (`cargo test -p craftcloud --test cli_live`): `login --kind
webdav`, `server check`, the file commands where two-way sync is allowed, and the "not available on this
server" answers. `CRAFTCLOUD_TEST_DAV_URL` names the WebDAV base where it isn't
`<server>/remote.php/dav/files/<user>/` (Nextcloud, ownCloud, OpenCloud) or the server address itself (plain WebDAV).

The check on connect (§ 7.5) alone, with every step printed:

```sh
CRAFTCLOUD_PROBE_URL=http://127.0.0.1:8115/dav/ CRAFTCLOUD_TEST_USER=john CRAFTCLOUD_TEST_PASSWORD="$DAV_PASS" \
  cargo test -p craftcloud-native probe_a_real_server -- --ignored --nocapture
```

Each test keeps to top-level folders of its own (`live-<test>-<seconds>-<pid>-…`) and deletes them, their
kept versions and the client-trash entries it made when it ends (on Nextcloud and ownCloud from the server's
trash too). What the check on connect leaves in the server's trash (one probe folder per probe, § 16, Q8)
stays, as it would for a person.

## In CI

The `other-servers` job of `.github/workflows/ci.yml` runs both suites against each service above, one matrix
entry per service: it starts only that service from this compose file with a random password made in the job,
waits for the test account's WebDAV folder to list, and runs `cli_live` and `live` with the variables above
(`CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` for that job only). It runs on pull requests and pushes that touch the sync,
the storage drivers, the live tests, this folder or the workflow, every night, and by hand
(`gh workflow run ci.yml`). A change to a service here (a new image digest, another port) is tried by that job on
its own pull request; a new service needs a matrix entry there.

## Tear down

```sh
docker compose -f tests/other-servers/compose.yaml -p craftcloud-others down -v
```

`-v` removes the volumes: these servers hold nothing worth keeping.

## The plain WebDAV servers, as measured

Measured on 2026-11-18 with the check on connect against the images pinned in `compose.yaml`:

| Behaviour | `rclone serve webdav` 1.76.0 | Apache httpd 2.4.71, `mod_dav` |
|---|---|---|
| `PUT` with `If-None-Match: *` over an existing file | **ignored: overwrites** (201) | 412 |
| `PUT` with a stale `If-Match` | **ignored: overwrites** (201) | 412 |
| `PUT`'s answer carries an ETag | yes | **no** (the listing's is used) |
| ETag changes on every write | yes (from the modified time) | yes for other bytes; the same bytes keep their tag (`FileETag Digest`) |
| ETag survives `MOVE` | yes | yes |
| `MOVE`/`COPY` with `Overwrite: F` onto an existing name | 412 | 412 |
| `DELETE` with a stale `If-Match` | **ignored: deletes** (204) | 412 |
| `PUT` into a missing folder | 409 | 409 |
| `Depth: infinity` | 207 | 403 |
| `oc:fileid`, `oc:checksums` | none | none |
| Mode | **download only** | **two-way**, client trash and versions |

Apache's default ETag (`FileETag MTime Size`) is marked weak (`W/"…"`) for a second after a write, and a
weak tag never matches `If-Match`: with it the check refused Apache at step 6 (overwriting the file it had
just written answered 412). `httpd.conf` therefore uses `FileETag Digest`, a strong tag from the content.

Apache also answers 301 to a `PROPFIND` or `DELETE` of a folder named without its trailing slash (mod_dir's
`DirectorySlash`, on by default). The first run of this suite found the generic driver following that redirect
as a `GET` without the `Authorization` header and reading the 401 as a revoked password ("Signed out: the
server no longer takes this app password" on a trashed folder's restore or purge); pull request 4b fixed it
(no redirect is ever followed, and a folder's URL carries its slash), so the server is kept as a person's
Apache would be, `DirectorySlash` on.
