"""Build design/craftcloud.figcraft and design/exports/*.png with FigCraft's headless engine.

    python3 -I design/tools/build.py /path/to/figcraft-cli

Everything in the document is made with FigCraft commands (see fc.py, lib.py); nothing writes the
.figcraft JSON directly.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib import DESIGN, Doc  # noqa: E402
from components import Library  # noqa: E402
import cards  # noqa: E402
import screens  # noqa: E402
import web  # noqa: E402
import v2  # noqa: E402
import v2_files  # noqa: E402  (Files: the page "v2 · Files")
import v2_sharing  # noqa: E402  (Sharing: the page "v2 · Sharing")
import v2_sso  # noqa: E402  (Single sign-on: the page "v2 · Single sign-on")
import v2_apps  # noqa: E402  (the Apps menu: the page "v2 · Apps")


KEEP_PAGES = ("Tokens & type", "Components", "v2 · Web", "v2 · Desktop",
              "v2 · Files", "v2 · Sharing", "v2 · Single sign-on", "v2 · Apps")  # one page per v2_*.py module


def archive(d):
    """Move every pre-v2 frame onto one Archive page (a row per old page) and drop the old pages."""
    pages = d.R("page.list")
    pages = pages["pages"] if isinstance(pages, dict) else pages
    arch = d.R("page.add", {"name": "Archive (pre-v2)"})["id"]
    y0 = 0
    for p in pages:
        if p["name"] in KEEP_PAGES:
            continue
        kids = d.R("node.get", {"id": p["id"], "fields": ["name", "y", "height"]}).get("children", [])
        bottom = max(k["y"] + k["height"] for k in kids)
        for k in kids:
            d.R("node.reparent", {"ids": [k["id"]], "parent": arch})
            d.set(k["id"], y=k["y"] + y0)
        y0 += bottom + 400
        d.R("page.delete", {"id": p["id"]})


def main():
    cli = sys.argv[1]
    only = sys.argv[2].split(",") if len(sys.argv) > 2 else None
    t0 = time.time()
    d = Doc(cli)
    p0 = d.page("Tokens & type", first=True)
    d.R("page.setBackground", {"id": p0, "color": "#0b0b0d"})
    d.tokens()
    screens.tokens_page(d)

    pc = d.page("Components")
    d.R("page.setBackground", {"id": pc, "color": "#0b0b0d"})
    lib = Library(d)
    lib.icons()
    lib.wordmark()
    lib.buttons()
    lib.badge()
    lib.cells()
    lib.inputs()
    lib.app_icon()
    lib.chrome()
    lib.toolbar()
    cards.card_component(d, lib)
    cards.table_row_component(d, lib)
    screens.more_components(d, lib)
    web.new_components(d, lib)
    v2.v2_components(d, lib)
    lib_frames = None  # noqa: F841

    old = screens.build_all(d, None)
    new = web.build_new(d)
    frames_v2 = v2.build_v2(d)
    frames_v2 += v2_files.build(d)  # Files: the page "v2 · Files"
    frames_v2 += v2_sharing.build(d)  # Sharing: the page "v2 · Sharing"
    frames_v2 += v2_sso.build(d)  # Single sign-on: the page "v2 · Single sign-on"
    frames_v2 += v2_apps.build(d)  # the Apps menu: the page "v2 · Apps"
    archive(d)
    out = os.path.join(DESIGN, "craftcloud.figcraft")
    d.save(out)
    print("saved", out, f"{time.time() - t0:.1f}s")
    ex = os.path.join(DESIGN, "exports")
    for sub in ("2x", "archive"):
        os.makedirs(os.path.join(ex, sub), exist_ok=True)
    for name, fid in frames_v2:
        if only and name not in only:
            continue
        d.export(fid, os.path.join(ex, f"{name}.png"), 1)
        d.export(fid, os.path.join(ex, "2x", f"{name}@2x.png"), 2)
    if not only:
        for name, fid in old + new:
            d.export(fid, os.path.join(ex, "archive", f"{name}.png"), 1)
    frames = frames_v2
    # Overview sheets of the Tokens and Components pages (not screens; one image each).
    for pname, fname in (("Tokens & type", "_tokens"), ("Components", "_components")):
        pages = d.R("page.list")
        pid = [p["id"] for p in (pages["pages"] if isinstance(pages, dict) else pages) if p["name"] == pname][0]
        kids = [k["id"] for k in d.R("node.get", {"id": pid, "fields": ["name"]}).get("children", [])]
        d.R("page.select", {"id": pid})
        d.export(kids, os.path.join(ex, f"{fname}.png"), 1)
    print("exported", len(frames), f"{time.time() - t0:.1f}s")
    d.fc.close()


if __name__ == "__main__":
    main()
