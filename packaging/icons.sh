#!/usr/bin/env bash
# Regenerate the desktop client's packaging icons from the CraftCloud mark (the mackerel),
# design/assets/logo/craftcloud-logo-1024.png. A new logo is this one command:
#
#   packaging/icons.sh [SOURCE.png]
#
# Writes (all committed, so packaging and the Rust build never need these tools):
#   packaging/linux/icons/hicolor/<n>x<n>/apps/io.github.jonasbove.craftcloud.png   16 … 512
#   packaging/macos/craftcloud.icns    16 … 1024, the mark on Apple's 824/1024 icon grid
#   apps/craftcloud/assets/craftcloud.ico   16 … 256; build.rs puts it in craftcloud.exe and the
#                                           MSI uses it for Apps & features
#
# The in-app PNGs (apps/craftcloud/assets/craftcloud-logo-*.png, crates/ui-egui/assets/) are not
# touched: the window, tray and snapshot tests use those as they are.
#
# Needs: python3 with Pillow (pip install pillow, or apt install python3-pil).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/design/assets/logo/craftcloud-logo-1024.png}"
[[ -f "$SRC" ]] || { echo "error: $SRC not found" >&2; exit 1; }
python3 -c 'import PIL' 2>/dev/null || { echo "error: python3 Pillow not found (pip install pillow)" >&2; exit 1; }

python3 - "$ROOT" "$SRC" <<'PY'
import os
import sys

from PIL import Image

root, src = sys.argv[1], sys.argv[2]
app_id = "io.github.jonasbove.craftcloud"
mark = Image.open(src).convert("RGBA")
if mark.width != mark.height or mark.width < 512:
    sys.exit(f"error: {src} must be square and at least 512 px (is {mark.width}x{mark.height})")


def sized(img, n):
    return img.resize((n, n), Image.Resampling.LANCZOS)


# Linux hicolor theme.
for n in (16, 24, 32, 48, 64, 128, 256, 512):
    out = os.path.join(root, "packaging/linux/icons/hicolor", f"{n}x{n}", "apps", f"{app_id}.png")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    sized(mark, n).save(out, optimize=True)

# Windows .ico: every size Explorer, the taskbar and Apps & features ask for.
ico_sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
ico = os.path.join(root, "apps/craftcloud/assets/craftcloud.ico")
big = sized(mark, 256)
big.save(ico, format="ICO", sizes=[(n, n) for n in ico_sizes],
         append_images=[sized(mark, n) for n in ico_sizes[:-1]])

# macOS .icns: Apple's grid leaves a transparent margin (the shape is 824 of 1024 px).
canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
canvas.alpha_composite(sized(mark, 824), (100, 100))
icns = os.path.join(root, "packaging/macos/craftcloud.icns")
canvas.save(icns, format="ICNS", append_images=[sized(canvas, n) for n in (16, 32, 64, 128, 256, 512)])
print(f"icons written from {src}")
PY
