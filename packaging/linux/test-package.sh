#!/usr/bin/env bash
# Install-test one Linux release file in a clean distro container, as a person would install it:
#
#   1. install it with the system's package manager, so its dependencies resolve
#      (apt-get for .deb, dnf for .rpm, pacman for .pkg.tar.zst; install.sh for the .tar.gz;
#      the AppImage runs extract-and-run, as there is no FUSE in a container; the .flatpak goes in
#      with `flatpak install --user --bundle`, its runtime from Flathub, in a privileged container,
#      as bubblewrap needs namespaces and mounts there)
#   2. `craftcloud --version` names the expected version and `craftcloud --help` works (the
#      Flatpak's inside its sandbox, through `flatpak run`)
#   3. the files are there: the binary on PATH, the desktop entry (desktop-file-validate), the icons,
#      the AppStream file and the craftcloud:// handler in mimeinfo.cache (the Flatpak's in its
#      exports, with the permissions the tray, keychain, notifications and sync folder need)
#   4. with --server: a sign-in without a browser or keychain (the device code is approved over
#      the API, as the /device page would), then mkdir, put, ls, get and `sync --once` into a
#      temporary sync folder, comparing the bytes both ways
#   5. with --previous: the previous release's package installed first, then upgraded to this one
#      (skipped, with a notice, when that release is not older)
#   6. removal: the files under /usr (or ~/.local for the .tar.gz and .flatpak) are gone again
#
# Usage, on a machine with Docker (the release workflow's package-test job runs exactly this):
#
#   packaging/linux/test-package.sh --image debian:12 [--server URL] [--previous OLD-FILE] FILE
#
#   FILE is a release file,
#   craftcloud-<version>-linux-<arch>.<deb|rpm|pkg.tar.zst|AppImage|tar.gz|flatpak>;
#   the version to expect is taken from its name. The container runs with the host's network, so
#   --server http://127.0.0.1:<port> reaches a server started on this machine, e.g. with
#   `KEEP=1 PROJECT=craftcloud-pkgtest scripts/smoke-test craftcloud:dev 8097`. The account is
#   CRAFTCLOUD_TEST_USER / CRAFTCLOUD_TEST_PASSWORD (as for the live tests); on a server without
#   an owner yet it is created as the owner. Use a throwaway server: the test leaves a folder
#   pkgtest-* in its trash.
set -euo pipefail

APP_ID=io.github.jonasbove.craftcloud
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"

IMAGE=""
SERVER=""
PREVIOUS=""
INSIDE=0
FILE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) IMAGE="$2"; shift 2 ;;
    --server) SERVER="${2%/}"; shift 2 ;;
    --previous) PREVIOUS="$2"; shift 2 ;;
    --inside) INSIDE=1; shift ;;
    -h | --help) sed -n '2,27p' "$SELF"; exit 0 ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) FILE="$1"; shift ;;
  esac
done
[[ -n "$FILE" ]] || { echo "usage: $0 --image IMAGE [--server URL] [--previous OLD-FILE] FILE" >&2; exit 2; }
if [[ -n "$SERVER" ]] && { [[ -z "${CRAFTCLOUD_TEST_USER:-}" ]] || [[ -z "${CRAFTCLOUD_TEST_PASSWORD:-}" ]]; }; then
  echo "error: --server needs CRAFTCLOUD_TEST_USER and CRAFTCLOUD_TEST_PASSWORD" >&2
  exit 2
fi

# ---- on the host: start the container and run this script in it ---------------------------------
if [[ "$INSIDE" = 0 ]]; then
  [[ -n "$IMAGE" ]] || { echo "error: --image is required (the test installs and removes system packages)" >&2; exit 2; }
  [[ -f "$FILE" ]] || { echo "error: $FILE not found" >&2; exit 1; }
  mounts=(-v "$SELF:/test-package.sh:ro" -v "$(cd "$(dirname "$FILE")" && pwd):/pkg/new:ro")
  args=(--inside)
  [[ -z "$SERVER" ]] || args+=(--server "$SERVER")
  if [[ -n "$PREVIOUS" ]]; then
    [[ -f "$PREVIOUS" ]] || { echo "error: $PREVIOUS not found" >&2; exit 1; }
    mounts+=(-v "$(cd "$(dirname "$PREVIOUS")" && pwd):/pkg/old:ro")
    args+=(--previous "/pkg/old/$(basename "$PREVIOUS")")
  fi
  # flatpak runs the app under bubblewrap, which needs to make namespaces and mounts: Docker's
  # default seccomp and AppArmor profiles forbid that, so the Flatpak's container is privileged.
  # It is a throwaway container, removed afterwards, that runs only the release file under test.
  run_opts=()
  [[ "$FILE" != *.flatpak ]] || run_opts+=(--privileged)
  echo "==> $(basename "$FILE") in $IMAGE"
  exec docker run --rm --network host --tmpfs /tmp:exec "${run_opts[@]}" \
    -e CRAFTCLOUD_TEST_USER -e CRAFTCLOUD_TEST_PASSWORD -e GITHUB_ACTIONS \
    "${mounts[@]}" "$IMAGE" bash /test-package.sh "${args[@]}" "/pkg/new/$(basename "$FILE")"
fi

# ---- in the container ----------------------------------------------------------------------------
# This installs and removes packages system-wide, so it refuses to run outside a container.
if [[ ! -f /.dockerenv && ! -f /run/.containerenv ]]; then
  echo "error: --inside runs only in a container; use --image" >&2
  exit 2
fi

notice() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::notice::$*"; else echo "notice: $*"; fi
  return
}
step() { echo; echo "--- $*"; return; }
ok() { echo "ok   $*"; return; }
fail() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::error::$*"; else echo "FAIL $*" >&2; fi
  exit 1
}

# The format and version from a release file's name.
format_of() {
  case "$1" in
    *.deb) echo deb ;;
    *.rpm) echo rpm ;;
    *.pkg.tar.zst) echo arch ;;
    *.AppImage) echo appimage ;;
    *.tar.gz) echo tar ;;
    *.flatpak) echo flatpak ;;
    *) return 1 ;;
  esac
}
version_of() { basename "$1" | sed -n 's/^craftcloud-\(.*\)-linux-[a-z0-9_]*\..*$/\1/p'; return; }

FMT="$(format_of "$FILE")" || fail "$FILE: not a .deb, .rpm, .pkg.tar.zst, .AppImage, .tar.gz or .flatpak"
VERSION="$(version_of "$FILE")"
[[ -n "$VERSION" ]] || fail "$FILE: the name is not craftcloud-<version>-linux-<arch>.<ext>"
if [[ "$FMT" = flatpak ]]; then
  # The sandbox has a /tmp of its own; what it shares is the home folder (--filesystem=home), so
  # the home folder and the round trip's files must not be under /tmp.
  mkdir -p /home
  WORK="$(mktemp -d /home/pkgtest.XXXXXX)"
else
  WORK="$(mktemp -d)"
fi
# Copies: apt reads the file as its own user, the AppImage must be executable, the mount is read-only.
cp "$FILE" "$WORK/"
PKG="$WORK/$(basename "$FILE")"
export HOME="$WORK/home"
mkdir -p "$HOME"
DATA="$HOME/.local/share"

# In a subshell: os-release sets VERSION too.
# shellcheck source=/dev/null
OS="$(. /etc/os-release && echo "${PRETTY_NAME:-$ID}")"
echo "==> CraftCloud $VERSION ($FMT) on $OS $(uname -m)"

# What to look at when something fails: the system, the package's state, and why the binary
# doesn't start (missing libraries).
debug_info() {
  echo
  echo "--- debug: $OS, $(uname -m), $(ldd --version 2>&1 | head -1)"
  case "$FMT" in
    deb) dpkg -l 'craftcloud*' 2>&1 | tail -n +6 || true ;;
    rpm) rpm -qi craftcloud 2>&1 | head -5 || true ;;
    arch) pacman -Qi craftcloud 2>&1 | head -5 || true ;;
    flatpak)
      flatpak --version 2>&1 || true
      flatpak list --user --columns=application,version,branch,runtime 2>&1 || true
      flatpak info --user --show-permissions "$APP_ID" 2>&1 || true
      ;;
  esac
  local bin=""
  case "$FMT" in
    deb | rpm | arch) bin=/usr/bin/craftcloud ;;
    tar) bin="$HOME/.local/bin/craftcloud" ;;
    appimage) (cd "$WORK" && "$PKG" --appimage-extract usr/bin/craftcloud >/dev/null 2>&1) && bin="$WORK/squashfs-root/usr/bin/craftcloud" ;;
  esac
  if [[ -n "$bin" && -f "$bin" ]]; then
    echo "--- ldd $bin"
    ldd "$bin" 2>&1 || true
  fi
  return
}
trap 'status=$?; if [[ $status -ne 0 ]]; then debug_info; fi' EXIT

# ---- 1. tools the checks need, from the distro (before the package, so its scripts see them) -----
step "test tools"
case "$FMT" in
  deb | appimage | tar)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends desktop-file-utils curl ca-certificates >/dev/null
    ;;
  rpm)
    dnf install -y -q --setopt=install_weak_deps=False desktop-file-utils
    command -v curl >/dev/null || dnf install -y -q curl
    ;;
  arch)
    # A full upgrade first: Arch has no partial upgrades, and the package's dependencies come
    # from today's repositories.
    pacman -Syu --noconfirm --needed desktop-file-utils curl >/dev/null
    ;;
  flatpak)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    # dbus: `flatpak run` needs the system bus (it fails with "Could not connect" without one) and a
    # session bus (its D-Bus proxy); a container has neither, so both are started here.
    apt-get install -y -qq --no-install-recommends flatpak dbus desktop-file-utils curl ca-certificates >/dev/null
    mkdir -p /run/dbus
    dbus-daemon --system --fork
    export XDG_RUNTIME_DIR=/run/user/0
    mkdir -p "$XDG_RUNTIME_DIR"
    chmod 700 "$XDG_RUNTIME_DIR"
    # The bundle names Flathub for its runtime (flatpak-bundle.sh's --runtime-repo).
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    ok "flatpak $(flatpak --version | cut -d' ' -f2), Flathub"
    ;;
esac
ok "desktop-file-utils, curl"

# ---- install, upgrade and remove -----------------------------------------------------------------
# The command to run, per format.
CC=()
case "$FMT" in
  deb | rpm | arch) CC=(craftcloud) ;;
  tar) CC=("$HOME/.local/bin/craftcloud") ;;
  appimage)
    chmod +x "$PKG"
    CC=(env APPIMAGE_EXTRACT_AND_RUN=1 CRAFTCLOUD_NO_DESKTOP_INTEGRATION=1 "$PKG")
    ;;
  # In its sandbox, each run with a session bus of its own; the CRAFTCLOUD_* variables the round
  # trip sets pass into it.
  flatpak) CC=(dbus-run-session -- flatpak run --command=craftcloud "$APP_ID") ;;
esac

install_file() { # file
  local file="$1"
  case "$FMT" in
    # --no-install-recommends / no weak deps: the hard dependencies are what has to resolve.
    deb) apt-get install -y -q --no-install-recommends "$file" ;;
    rpm) dnf install -y -q --setopt=install_weak_deps=False "$file" ;;
    arch) pacman -U --noconfirm "$file" ;;
    tar)
      rm -rf "$WORK/unpacked"
      mkdir -p "$WORK/unpacked"
      tar -xzf "$file" -C "$WORK/unpacked"
      "$WORK/unpacked/$(basename "$file" .tar.gz)/install.sh"
      ;;
    appimage) ;;
    # --reinstall: a bundle of the same app over an installed one is how a bundle updates.
    flatpak) flatpak install --user -y --noninteractive --reinstall --bundle "$file" ;;
  esac
  return
}

remove_package() {
  case "$FMT" in
    deb) apt-get purge -y -q craftcloud ;;
    rpm) dnf remove -y -q craftcloud ;;
    arch) pacman -R --noconfirm craftcloud ;;
    tar) "$WORK/unpacked/$(basename "$PKG" .tar.gz)/install.sh" --remove ;;
    appimage) ;;
    flatpak) flatpak uninstall --user -y --noninteractive "$APP_ID" ;;
  esac
  return
}

check_version() { # expected version
  local want="$1" out
  # Standard output only: `flatpak run` may warn on standard error (no session bus in a container).
  if ! out="$("${CC[@]}" --version 2>"$WORK/version.err")"; then
    cat "$WORK/version.err"
    echo "$out"
    fail "craftcloud --version didn't run"
  fi
  [[ "$out" = "craftcloud $want" ]] || fail "craftcloud --version said '$out', expected 'craftcloud $want'"
  ok "--version: $out"
  "${CC[@]}" --help >/dev/null || fail "craftcloud --help failed"
  ok "--help"
  return
}

# Where the files go: /usr for packages, ~/.local for install.sh.
if [[ "$FMT" = tar ]]; then PREFIX_BIN="$HOME/.local/bin" PREFIX_SHARE="$DATA"; else PREFIX_BIN=/usr/bin PREFIX_SHARE=/usr/share; fi

check_files() {
  [[ -x "$PREFIX_BIN/craftcloud" ]] || fail "$PREFIX_BIN/craftcloud missing"
  if [[ "$FMT" != tar ]]; then
    # Resolved: on Fedora 42 /usr/sbin is a link to /usr/bin and comes first on PATH.
    [[ "$(readlink -f "$(command -v craftcloud)")" = "$(readlink -f /usr/bin/craftcloud)" ]] ||
      fail "craftcloud on PATH is '$(command -v craftcloud)'"
  fi
  ok "binary $PREFIX_BIN/craftcloud"
  local desktop="$PREFIX_SHARE/applications/$APP_ID.desktop"
  [[ -f "$desktop" ]] || fail "$desktop missing"
  desktop-file-validate "$desktop" || fail "$desktop doesn't validate"
  ok "desktop entry validates"
  local icons=("$PREFIX_SHARE"/icons/hicolor/*/apps/"$APP_ID".png)
  [[ -f "$PREFIX_SHARE/icons/hicolor/256x256/apps/$APP_ID.png" && "${#icons[@]}" -ge 8 ]] ||
    fail "icons: found ${icons[*]}"
  ok "${#icons[@]} icons"
  local meta="$PREFIX_SHARE/metainfo/$APP_ID.metainfo.xml"
  grep -q "<release version=\"$CURRENT_VERSION\"" "$meta" 2>/dev/null || fail "$meta missing or not for $CURRENT_VERSION"
  ok "AppStream file"
  # update-desktop-database (the package's postinst, the distro's trigger or install.sh) writes the
  # entry's MimeType into mimeinfo.cache, which is how craftcloud:// links find it.
  grep -qx "x-scheme-handler/craftcloud=.*$APP_ID.desktop.*" "$PREFIX_SHARE/applications/mimeinfo.cache" 2>/dev/null ||
    fail "x-scheme-handler/craftcloud is not in $PREFIX_SHARE/applications/mimeinfo.cache"
  ok "craftcloud:// handler registered"
  return
}

check_removed() {
  local left=()
  [[ ! -e "$PREFIX_BIN/craftcloud" ]] || left+=("$PREFIX_BIN/craftcloud")
  [[ ! -e "$PREFIX_SHARE/applications/$APP_ID.desktop" ]] || left+=("$PREFIX_SHARE/applications/$APP_ID.desktop")
  [[ ! -e "$PREFIX_SHARE/metainfo/$APP_ID.metainfo.xml" ]] || left+=("$PREFIX_SHARE/metainfo/$APP_ID.metainfo.xml")
  local icon
  for icon in "$PREFIX_SHARE"/icons/hicolor/*/apps/"$APP_ID".png; do
    [[ ! -e "$icon" ]] || left+=("$icon")
  done
  [[ "${#left[@]}" -eq 0 ]] || fail "left behind after removal: ${left[*]}"
  if grep -q "$APP_ID.desktop" "$PREFIX_SHARE/applications/mimeinfo.cache" 2>/dev/null; then
    fail "mimeinfo.cache still names $APP_ID.desktop after removal"
  fi
  ok "removed: binary, desktop entry, icons, AppStream file, handler"
  return
}

# The AppImage installs nothing, but its AppRun adds the menu entry and icons to ~/.local/share on
# its first run (packaging/linux/AppRun), so a Wayland compositor finds the window's icon.
check_appimage_integration() {
  APPIMAGE_EXTRACT_AND_RUN=1 "$PKG" --version >/dev/null || fail "the AppImage didn't start"
  local desktop="$DATA/applications/$APP_ID.desktop"
  [[ -f "$desktop" ]] || fail "the AppImage's first run didn't add $desktop"
  desktop-file-validate "$desktop" || fail "$desktop doesn't validate"
  grep -q "^Exec=\"$PKG\" %u" "$desktop" || fail "$desktop doesn't start the AppImage: $(grep '^Exec=' "$desktop")"
  [[ -f "$DATA/icons/hicolor/256x256/apps/$APP_ID.png" ]] || fail "the AppImage's first run didn't add its icons"
  ok "first run added the menu entry and icons to ~/.local/share"
  # update-desktop-database (desktop-file-utils, installed above) is what AppRun runs for this.
  grep -qx "x-scheme-handler/craftcloud=.*$APP_ID.desktop.*" "$DATA/applications/mimeinfo.cache" 2>/dev/null ||
    fail "x-scheme-handler/craftcloud is not in $DATA/applications/mimeinfo.cache after the first run"
  ok "craftcloud:// handler registered"
  return
}

# The Flatpak: installed for the user, its menu entry, icons and handler exported to
# ~/.local/share/flatpak/exports, its AppStream file in the app, and the permissions it ships with.
FLATPAK_EXPORTS="$DATA/flatpak/exports/share"
check_flatpak_files() {
  flatpak info --user "$APP_ID" >/dev/null || fail "flatpak info doesn't know $APP_ID"
  ok "installed: $(flatpak info --user "$APP_ID" | grep -E '^ *(Version|Runtime):' | tr -s ' ' | tr '\n' ' ')"
  local desktop="$FLATPAK_EXPORTS/applications/$APP_ID.desktop"
  [[ -f "$desktop" ]] || fail "$desktop missing"
  desktop-file-validate "$desktop" || fail "$desktop doesn't validate"
  grep -q "^Exec=.*flatpak run .*$APP_ID" "$desktop" || fail "$desktop doesn't start the Flatpak: $(grep '^Exec=' "$desktop")"
  ok "exported desktop entry validates and runs the Flatpak"
  local icons=("$FLATPAK_EXPORTS"/icons/hicolor/*/apps/"$APP_ID".png)
  [[ -f "$FLATPAK_EXPORTS/icons/hicolor/256x256/apps/$APP_ID.png" && "${#icons[@]}" -ge 8 ]] ||
    fail "icons: found ${icons[*]}"
  ok "${#icons[@]} icons exported"
  local meta
  meta="$(flatpak info --user --show-location "$APP_ID")/files/share/metainfo/$APP_ID.metainfo.xml"
  grep -q "<release version=\"$CURRENT_VERSION\"" "$meta" 2>/dev/null || fail "$meta missing or not for $CURRENT_VERSION"
  ok "AppStream file"
  # flatpak's desktop-database trigger runs update-desktop-database on the exports.
  grep -qx "x-scheme-handler/craftcloud=.*$APP_ID.desktop.*" "$FLATPAK_EXPORTS/applications/mimeinfo.cache" 2>/dev/null ||
    fail "x-scheme-handler/craftcloud is not in $FLATPAK_EXPORTS/applications/mimeinfo.cache"
  ok "craftcloud:// handler registered"
  # What the tray, the keychain, the notifications and the sync folder need (the manifest's
  # finish-args); losing one breaks the client only on a desktop, where no test here looks.
  local perms want
  perms="$(flatpak info --user --show-permissions "$APP_ID")"
  for want in org.kde.StatusNotifierWatcher=talk org.freedesktop.secrets=talk org.freedesktop.Notifications=talk; do
    grep -qx "$want" <<<"$perms" || fail "permission $want missing: $perms"
  done
  grep -q '^filesystems=.*\bhome\b' <<<"$perms" || fail "permission filesystems=home missing: $perms"
  ok "permissions: tray, keychain, notifications, home folder"
  return
}

check_flatpak_removed() {
  if flatpak info --user "$APP_ID" >/dev/null 2>&1; then fail "$APP_ID still installed after removal"; fi
  local left=()
  [[ ! -e "$FLATPAK_EXPORTS/applications/$APP_ID.desktop" ]] || left+=("$FLATPAK_EXPORTS/applications/$APP_ID.desktop")
  local icon
  for icon in "$FLATPAK_EXPORTS"/icons/hicolor/*/apps/"$APP_ID".png; do
    [[ ! -e "$icon" ]] || left+=("$icon")
  done
  [[ "${#left[@]}" -eq 0 ]] || fail "left behind after removal: ${left[*]}"
  if grep -q "$APP_ID.desktop" "$FLATPAK_EXPORTS/applications/mimeinfo.cache" 2>/dev/null; then
    fail "mimeinfo.cache still names $APP_ID.desktop after removal"
  fi
  ok "removed: the app, its exported desktop entry, icons and handler"
  return
}

check_installed() {
  case "$FMT" in
    appimage) check_appimage_integration ;;
    flatpak) check_flatpak_files ;;
    *) check_files ;;
  esac
  return
}

check_gone() {
  if [[ "$FMT" = flatpak ]]; then check_flatpak_removed; else check_removed; fi
  return
}

# ---- 4. a sync round trip against a real server ---------------------------------------------------
json_field() { # name → the first string value of "name" in stdin's JSON
  grep -o "\"$1\" *: *\"[^\"]*\"" | head -1 | sed 's/.*: *"\(.*\)"$/\1/'
  return
}

# The same bytes (sha256sum: coreutils is in every base image, cmp is not).
same() { # a b
  [[ -f "$1" && -f "$2" && "$(sha256sum <"$1")" = "$(sha256sum <"$2")" ]]
}

round_trip() {
  local user="$CRAFTCLOUD_TEST_USER" password="$CRAFTCLOUD_TEST_PASSWORD"
  local rt="$HOME/round-trip"
  local sync="$rt/CraftCloud"
  mkdir -p "$rt" "$sync"
  # Settings and tokens in a folder of their own: no keychain in a container, and nothing of the
  # person's own (CRAFTCLOUD_KEYCHAIN=off keeps the tokens in a private file there).
  export CRAFTCLOUD_CONFIG_DIR="$rt/config" CRAFTCLOUD_KEYCHAIN=off
  unset CRAFTCLOUD_SERVER

  local status
  status="$(curl -fsS "$SERVER/api/auth/status")" || fail "$SERVER/api/auth/status doesn't answer"
  if printf '%s' "$status" | grep -q '"initialized" *: *false'; then
    curl -fsS -o /dev/null -H 'Content-Type: application/json' \
      -d "{\"username\":\"$user\",\"email\":\"$user@example.org\",\"password\":\"$password\"}" \
      "$SERVER/api/setup" || fail "couldn't create the owner"
    ok "created the owner $user"
  fi

  # Sign in as a person without a browser would: `login --no-browser` prints the code, and the code
  # is approved with a password sign-in, as the /device page does.
  "${CC[@]}" login "$SERVER" --no-browser --json >"$rt/login.out" 2>"$rt/login.err" &
  local pid=$! i code token
  for ((i = 0; i < 300; i++)); do
    [[ -s "$rt/login.out" ]] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  code="$(head -1 "$rt/login.out" | json_field user_code)"
  [[ -n "$code" ]] || { cat "$rt/login.out" "$rt/login.err"; kill "$pid" 2>/dev/null || true; fail "login printed no code"; }
  token="$(curl -fsS -H 'Content-Type: application/json' -d "{\"username\":\"$user\",\"password\":\"$password\"}" \
    "$SERVER/api/auth/login" | json_field access_token)"
  [[ -n "$token" ]] || { kill "$pid" 2>/dev/null || true; fail "password sign-in to approve the code failed"; }
  curl -fsS -o /dev/null -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
    -d "{\"user_code\":\"$code\",\"action\":\"approve\"}" "$SERVER/api/auth/device/verify" ||
    { kill "$pid" 2>/dev/null || true; fail "approving the device code failed"; }
  for ((i = 0; i < 600; i++)); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    fail "login still waiting 60 s after the code was approved"
  fi
  wait "$pid" || { cat "$rt/login.out" "$rt/login.err"; fail "login failed"; }
  sed -n 2p "$rt/login.out" | grep -q '"signed_in":true' || { cat "$rt/login.out"; fail "login didn't sign in"; }
  ok "signed in through the device flow (no browser, no keychain)"

  # The sync folder, as the desktop client's setup would choose it.
  local config="$CRAFTCLOUD_CONFIG_DIR/config.toml"
  [[ -f "$config" ]] || fail "login wrote no $config"
  if grep -q '^local_folder *=' "$config"; then
    sed -i "s|^local_folder *=.*|local_folder = \"$sync\"|" "$config"
  else
    printf 'local_folder = "%s"\n' "$sync" >>"$config"
  fi

  local dir="pkgtest-$FMT-$RANDOM$RANDOM" out
  cc() { "${CC[@]}" "$@" || fail "craftcloud $* failed (exit $?)"; return; }
  cc mkdir "$dir" >/dev/null
  head -c 300000 /dev/urandom >"$rt/up.bin"
  cc put "$rt/up.bin" "$dir/" >/dev/null
  out="$(cc ls "$dir" --json)"
  printf '%s' "$out" | grep -q "\"path\":\"$dir/up.bin\"" || fail "ls $dir: $out"
  cc get "$dir/up.bin" "$rt/down.bin" >/dev/null
  same "$rt/up.bin" "$rt/down.bin" || fail "get: the bytes differ from put's"
  ok "mkdir, put, ls, get: the same bytes"

  out="$("${CC[@]}" sync --once --json)" || { echo "$out"; fail "sync --once failed (download)"; }
  same "$rt/up.bin" "$sync/$dir/up.bin" || { echo "$out"; fail "sync --once: the file put from the command line isn't in the sync folder"; }
  ok "sync --once downloaded the file put from the command line"

  head -c 200000 /dev/urandom >"$sync/$dir/local.bin"
  # The sync leaves a file written in the last 2 seconds for the next pass (an app may still be
  # saving it: SETTLE_MS in crates/native/src/sync.rs).
  sleep 3
  out="$("${CC[@]}" sync --once --json)" || { echo "$out"; fail "sync --once failed (upload)"; }
  printf '%s' "$out" | grep -q '"state":"up_to_date"' || { echo "$out"; fail "sync --once didn't end up to date"; }
  out="$(cc ls "$dir" --json)"
  printf '%s' "$out" | grep -q "\"path\":\"$dir/local.bin\"" || fail "after sync, ls $dir: $out"
  cc get "$dir/local.bin" "$rt/local-down.bin" >/dev/null
  same "$sync/$dir/local.bin" "$rt/local-down.bin" || fail "a file written into the sync folder came back different"
  ok "sync --once uploaded a file written into the sync folder: the same bytes"

  cc rm "$dir" >/dev/null
  cc logout >/dev/null
  unset CRAFTCLOUD_CONFIG_DIR CRAFTCLOUD_KEYCHAIN
  return
}

CURRENT_VERSION="$VERSION"

step "2-3. install $(basename "$PKG")"
install_file "$PKG"
check_version "$VERSION"
check_installed

if [[ -n "$SERVER" ]]; then
  step "4. sync round trip against $SERVER"
  round_trip
else
  notice "no --server: the sync round trip was skipped"
fi

if [[ "$FMT" != appimage ]]; then
  step "6. remove"
  remove_package
  check_gone
fi

# ---- 5. upgrade from the previous release --------------------------------------------------------
# GNU sort -V sorts `~` first: 0.2.0-rc.1 written as 0.2.0~rc.1 sorts before 0.2.0, as in semver.
older() { # a b → a sorts before b
  local a="${1//-/\~}" b="${2//-/\~}"
  [[ "$a" != "$b" && "$(printf '%s\n%s\n' "$a" "$b" | sort -V | head -1)" = "$a" ]]
}
if [[ -n "$PREVIOUS" ]]; then
  if [[ "$FMT" = appimage || "$FMT" = tar ]]; then
    notice "$FMT: nothing is installed, so there is no upgrade to test"
  else
    OLD_VERSION="$(version_of "$PREVIOUS")"
    if ! older "$OLD_VERSION" "$VERSION"; then
      notice "the previous release ($OLD_VERSION) is not older than $VERSION: upgrade not tested"
    else
      step "5. upgrade from $OLD_VERSION"
      cp "$PREVIOUS" "$WORK/"
      install_file "$WORK/$(basename "$PREVIOUS")"
      check_version "$OLD_VERSION"
      install_file "$PKG"
      check_version "$VERSION"
      check_installed
      step "6. remove after the upgrade"
      remove_package
      check_gone
    fi
  fi
else
  notice "no --previous: the upgrade was not tested"
fi

echo
echo "==> all checks passed: CraftCloud $VERSION ($FMT) on $OS $(uname -m)"
