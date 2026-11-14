#!/usr/bin/env bash
# Build the release's Arch Linux package from the Linux tarball, with makepkg:
#
#   $DIST/craftcloud-<version>-linux-<arch>.pkg.tar.zst     (pacman -U it)
#
# Usage: packaging/arch/package.sh [TARBALL] [ARCH]
#
# TARBALL defaults to $DIST/craftcloud-<version>-linux-<ARCH>.tar.gz from packaging/linux/package.sh,
# ARCH to the host's (x86_64 or aarch64). It only repackages the tarball's bin/ and share/, so an
# x86_64 host can build the aarch64 package. Needs makepkg (Arch, or the archlinux:base-devel
# container as an unprivileged user: makepkg refuses to run as root).
#
# The same steps as craft-common-actions' arch-package (MIT OR Apache-2.0; see NOTICE), with
# CraftCloud's own dependencies. The AUR recipes are in craftcloud-bin/ and craftcloud/.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
PKG=craftcloud

ARCH="${2:-$(uname -m)}"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  riscv64) ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
TARBALL="${1:-$DIST/$PKG-$VERSION-linux-$ARCH.tar.gz}"
[[ -f "$TARBALL" ]] || { echo "error: $TARBALL not found (run packaging/linux/package.sh --formats tar first)" >&2; exit 1; }
for tool in makepkg bsdtar zstd; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found" >&2; exit 1; }
done
TARBALL="$(cd "$(dirname "$TARBALL")" && pwd)/$(basename "$TARBALL")"

# pacman's pkgver has no hyphens: 0.2.0-rc.1 becomes 0.2.0rc.1, which sorts before 0.2.0. A
# numeric pre-release needs a letter in front (0.2.0-1 as 0.2.01 would sort after 0.2.0).
PKGVER="$VERSION"
if [[ "$PKGVER" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9] ]]; then PKGVER="${PKGVER/-/pre}"; fi
PKGVER="${PKGVER//-/}"
[[ "$PKGVER" =~ ^[A-Za-z0-9._+]+$ ]] || { echo "error: can't make a pkgver from $VERSION" >&2; exit 1; }

WORK="$CARGO_TARGET_DIR/arch-package"
rm -rf "$WORK"
mkdir -p "$WORK/stage"
# Absolute: makepkg runs in $WORK, and the PKGBUILD names files in it.
WORK="$(cd "$WORK" && pwd)"
tar -xzf "$TARBALL" -C "$WORK/stage" --strip-components=1
# -f, not -x: the work dir may be on a noexec mount (a container tmpfs).
[[ -f "$WORK/stage/bin/$PKG" ]] || { echo "error: $TARBALL has no bin/$PKG" >&2; exit 1; }

cat >"$WORK/makepkg.conf" <<EOF
if [[ -f /etc/makepkg.conf ]]; then source /etc/makepkg.conf; fi
CARCH=$ARCH
CHOST=$ARCH-unknown-linux-gnu
PKGEXT='.pkg.tar.zst'
COMPRESSZST=(zstd -c -T0 -19 -)
EOF

# The windowing and GPU libraries are loaded at runtime (dlopen), so makepkg can't find them.
# The tray, keychain and notifications are pure-Rust D-Bus: no libdbus or GTK.
cat >"$WORK/PKGBUILD" <<EOF
# Maintainer: CraftCloud contributors <https://github.com/jonasbove/craftcloud>
pkgname=$PKG
pkgver=$PKGVER
pkgrel=1
pkgdesc='Desktop sync client and command line for a CraftCloud server'
arch=($ARCH)
url='https://github.com/jonasbove/craftcloud'
license=('MIT OR Apache-2.0')
depends=('glibc' 'gcc-libs' 'hicolor-icon-theme' 'libxkbcommon' 'libxkbcommon-x11' 'xdg-utils')
optdepends=(
  'org.freedesktop.secrets: keep the sign-in in the keychain (gnome-keyring, kwallet, keepassxc)'
  'wayland: native Wayland windows'
  'libx11: X11 and XWayland windows'
  'libxcursor: X11 cursors'
  'libxi: X11 input'
  'vulkan-icd-loader: GPU drawing through Vulkan'
  'libglvnd: GPU drawing through EGL'
)
provides=('craftcloud')
conflicts=('craftcloud-bin' 'craftcloud-git')
options=('!strip' '!debug')

package() {
  install -Dm755 "$WORK/stage/bin/$PKG" "\$pkgdir/usr/bin/$PKG"
  cp -dr --no-preserve=ownership "$WORK/stage/share" "\$pkgdir/usr/"
  install -Dm644 "$WORK/stage/share/doc/$PKG/LICENSE-MIT" "\$pkgdir/usr/share/licenses/$PKG/LICENSE-MIT"
  install -Dm644 "$WORK/stage/share/doc/$PKG/LICENSE-APACHE" "\$pkgdir/usr/share/licenses/$PKG/LICENSE-APACHE"
}
EOF

echo "==> Arch package $PKG $PKGVER-1 for $ARCH"
(cd "$WORK" && SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || date +%s)}" \
  makepkg --config "$WORK/makepkg.conf" --nodeps --clean --force)
shopt -s nullglob
built=("$WORK"/*.pkg.tar.zst)
shopt -u nullglob
[[ "${#built[@]}" -eq 1 ]] || { echo "error: expected one package, found ${#built[@]}" >&2; exit 1; }
OUT="$DIST/$PKG-$VERSION-linux-$ARCH.pkg.tar.zst"
cp -f "${built[0]}" "$OUT"
echo "wrote $OUT"
