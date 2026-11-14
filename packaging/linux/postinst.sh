#!/bin/sh
# deb/rpm post-install and post-remove: refresh the desktop, MIME and icon caches, so the menu
# shows CraftCloud and craftcloud:// links open it (the desktop entry's
# MimeType=x-scheme-handler/craftcloud; update-desktop-database writes that into mimeinfo.cache).
# Every tool is optional; missing ones are skipped. Per-user choices (xdg-mime default) are left
# to the person: a system package doesn't write into home folders.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
