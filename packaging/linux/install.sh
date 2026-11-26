#!/bin/sh
# Adds CraftCloud to your app menu (no root needed): the binary to ~/.local/bin, the menu entry,
# the AppStream file and icons to ~/.local/share. Run from the unpacked release tarball
# (craftcloud-<ver>-linux-<arch>/): ./install.sh   Undo: ./install.sh --remove
#
# Earlier versions installed the menu entry as craftcloud.desktop with craftcloud.png icons;
# both directions remove those, so the menu never lists CraftCloud twice.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
bin="${HOME}/.local/bin"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
id=io.github.jonasbove.craftcloud
sizes="16 24 32 48 64 128 256 512"

remove_old() {
    rm -f "$data/applications/craftcloud.desktop"
    for s in $sizes; do rm -f "$data/icons/hicolor/${s}x${s}/apps/craftcloud.png"; done
}

refresh() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$data/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
    fi
}

if [ "${1:-}" = --remove ]; then
    rm -f "$bin/craftcloud" "$data/applications/$id.desktop" "$data/metainfo/$id.metainfo.xml"
    for s in $sizes; do rm -f "$data/icons/hicolor/${s}x${s}/apps/$id.png"; done
    remove_old
    refresh
    echo "Removed CraftCloud from the app menu. Your synced folder and settings are untouched."
    exit 0
fi

[ -x "$here/bin/craftcloud" ] || { echo "error: run this from the unpacked craftcloud-<ver>-linux-<arch> folder" >&2; exit 1; }
install -Dm755 "$here/bin/craftcloud" "$bin/craftcloud"
# The menu entry starts the installed copy by its full path, so ~/.local/bin needn't be on PATH.
tmp="$data/applications/.$id.desktop.tmp"
mkdir -p "$data/applications"
sed -e "s|^Exec=craftcloud|Exec=\"$bin/craftcloud\"|" -e "s|^TryExec=.*|TryExec=$bin/craftcloud|" \
    "$here/share/applications/$id.desktop" > "$tmp"
mv -f "$tmp" "$data/applications/$id.desktop"
install -Dm644 "$here/share/metainfo/$id.metainfo.xml" "$data/metainfo/$id.metainfo.xml"
for s in $sizes; do
    icon="$here/share/icons/hicolor/${s}x${s}/apps/$id.png"
    if [ -f "$icon" ]; then install -Dm644 "$icon" "$data/icons/hicolor/${s}x${s}/apps/$id.png"; fi
done
remove_old
refresh
# craftcloud:// links (the entry's MimeType=x-scheme-handler/craftcloud) open with this client.
if command -v xdg-mime >/dev/null 2>&1; then
    xdg-mime default "$id.desktop" x-scheme-handler/craftcloud 2>/dev/null || true
fi
echo "Installed: CraftCloud is in your app menu (binary: $bin/craftcloud)."
