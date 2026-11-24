"""v2 · Apps (0.3.0): the Apps menu in the web header, for the Craft apps the server hosts in the browser.

When the server has the Craft web apps on and at least one app switched on (`/craftcloud/config.json`'s
`web_apps` and `enabled_apps`), a signed-in person's header gets an APPS cell between FILES and ADMIN: the
app-grid glyph and the word (the glyph alone on a phone). Clicking it drops a popover from the header, like
the account menu: a grid of app tiles, four to a row, each opening `/apps/<id>/` in a new tab. With the
apps off, or none switched on, the header is as before.

The frames show Home with the menu open (dark, light) and the same on a phone. The apps are the ones the
Admin › Server frames switch on (v2.WEB_ON), in apps.toml's order.
"""

from lib import APP
from components import ICON
from screens import hud, side_stroke, screen, vstack
import v2
from v2 import W, GAP, OWNER

PAGE = "v2 · Apps"
PHONE_W, PHONE_H = 390, 844
# apps.toml's order (the order /craftcloud/config.json lists enabled_apps in), the apps v2 switches on.
APPS_ON = [a for a in ("wordcraft", "gridcraft", "deckcraft", "pdfcraft", "figcraft", "photocraft") if a in v2.WEB_ON]

# The menu: 4 tiles to a row, 96 × 88 each, 8 px around the grid.
COLS, TILE_W, TILE_H, GRID_PAD = 4, 96, 88, 8
MENU_W = COLS * TILE_W + 2 * GRID_PAD  # 400
TILE_ICON = 40
# The app-grid glyph: nine 2 px squares in a 14 px box (drawn solid).
GRID = " ".join(f"M{x} {y} L{x + 2.5} {y} L{x + 2.5} {y + 2.5} L{x} {y + 2.5} Z"
                for y in (1.5, 5.75, 10) for x in (1.5, 5.75, 10))
EXTERNAL = v2.ICONS["external"]


def glyph(d, parent, path, color, name, solid=False, size=14):
    box = d.frame(parent, name, 0, 0, size, size, fill=None, clip=False)
    d.node("vector", box, "glyph", (size - 14) / 2, (size - 14) / 2, path=path, closed=solid,
           fill=color if solid else None, stroke=None if solid else color, strokeWeight=1.5)
    return box


def apps_cell(d, nav, compact=False):
    """The APPS cell, open: sunken, ink_strong, the grid glyph (and the word on wide headers)."""
    c = d.frame(nav, "Nav Apps", 0, 0, 10, 24, fill="sunken", layout="horizontal", gap=6,
                padding=[0, 5 if compact else 8, 0, 5 if compact else 8], crossAlign="center", sizingH="hug",
                sizingV="fixed", clip=False)
    glyph(d, c, GRID, "ink_strong", "Grid glyph", solid=True)
    if not compact:
        hud(d, c, "Apps", color="ink_strong", name="Label")
    return c


def insert_after(d, parent, node, after_name):
    """Moves `node` to just after the child `after_name` of `parent`."""
    kids = d.R("node.get", {"id": parent, "fields": ["name"]}).get("children", [])
    names = [k["name"] for k in kids]
    d.R("node.reparent", {"ids": [node], "parent": parent, "index": names.index(after_name) + 1})


def tile(d, parent, app, hover=False):
    a = APP[app]
    t = d.frame(parent, f"Tile {a['name']}", 0, 0, TILE_W, TILE_H, fill="sunken" if hover else None,
                layout="vertical", gap=8, padding=[14, 6, 10, 6], crossAlign="center", sizingH="fixed",
                sizingV="fixed", clip=True)
    ic = d.inst("App icon", t, name="Icon")
    d.set(ic, width=TILE_ICON, height=TILE_ICON)
    d.R("image.replace", {"ids": [d.sub(ic, "Icon image")], "path": ICON(app)})
    n = d.text(t, a["name"], "Small/12", "ink_strong" if hover else "ink", name="Name", align="center")
    d.set(n, sizingH="fill", textAutoResize="truncate", maxLines=1)
    if hover:
        # The card's hover: a 2 px rule in the app's colour along the top.
        r = d.rect(t, 0, 0, TILE_W, 2, f"app:{app}/color", name="Hover rule")
        d.set(r, absolute=True)
        d.set(r, x=0, y=0)
    return t


def apps_menu(d, f, x, y, w=MENU_W, hover="wordcraft"):
    """The popover: who it's for, the tiles, and the new-tab note."""
    m = d.frame(f, "Apps menu", x, y, w, 10, fill="bg", stroke="line_strong", layout="vertical", sizingV="hug",
                clip=False)
    d.effect_style(m, "Shadow/Dialog")
    head = d.frame(m, "Head", 0, 0, w, 40, fill=None, layout="horizontal", padding=[0, 14, 0, 14],
                   crossAlign="center", sizingH="fill", sizingV="fixed")
    side_stroke(d, head, bottom=1)
    hud(d, head, "Craft apps in the browser", color="ink", name="Title")
    d.frame(head, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    hud(d, head, str(len(APPS_ON)), color="muted", name="Count")
    tile_w = (w - 2 * GRID_PAD) / COLS
    grid = d.frame(m, "Grid", 0, 0, w, 10, fill=None, layout="vertical", padding=[GRID_PAD] * 4, sizingH="fill",
                   sizingV="hug", clip=False)
    for r0 in range(0, len(APPS_ON), COLS):
        row = d.frame(grid, f"Row {r0 // COLS + 1}", 0, 0, w - 2 * GRID_PAD, TILE_H, fill=None, layout="horizontal",
                      sizingH="fill", sizingV="fixed", clip=False)
        for app in APPS_ON[r0:r0 + COLS]:
            t = tile(d, row, app, hover=app == hover)
            d.set(t, width=tile_w)
            if app == hover:
                d.set(d.sub(t, "Hover rule"), width=tile_w)
    foot = d.frame(m, "Foot", 0, 0, w, 34, fill=None, layout="horizontal", gap=8, padding=[0, 14, 0, 14],
                   crossAlign="center", sizingH="fill", sizingV="fixed")
    side_stroke(d, foot, top=1)
    d.text(foot, "Each app opens in a new tab.", "Small/12", "muted", name="Note")
    d.frame(foot, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    glyph(d, foot, EXTERNAL, "muted", "External")
    return m


def abs_bounds(d, node):
    return d.R("node.get", {"id": node, "fields": ["absoluteBounds"]})["absoluteBounds"]


def menu_frame(d, name, x, y, light=False):
    """Home with the Apps menu open under the APPS cell."""
    f = v2.v2_home(d, name, x, y, light=light)
    hid = d.sub(f, "Header")
    d.R("component.detach", {"ids": [hid]})
    nav = d.sub(hid, "Nav")
    cell = apps_cell(d, nav)
    insert_after(d, nav, cell, "Nav Files")
    cb, fb = abs_bounds(d, cell), abs_bounds(d, f)
    apps_menu(d, f, cb["x"] - fb["x"], 48 + 4)
    return f


def phone_frame(d, name, x, y, light=False):
    """A phone: the header as the Files phone frames have it, APPS as the glyph alone, the menu 8 px a side."""
    f = screen(d, name, x, y, PHONE_W, PHONE_H, light=light)
    hd = d.frame(f, "Header", 0, 0, PHONE_W, 48, fill="bg", layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1)

    def cell(nm, pad=(0, 12, 0, 12), right=False, left=False, fixed=None):
        c = d.frame(hd, nm, 0, 0, fixed or 10, 48, fill=None, layout="horizontal", padding=list(pad), gap=4,
                    crossAlign="center", sizingH="fixed" if fixed else "hug", sizingV="fill", clip=False)
        if fixed:
            d.set(c, mainAlign="center")
        if left or right:
            side_stroke(d, c, left=1 if left else 0, right=1 if right else 0)
        return c

    b = cell("Brand", (0, 14, 0, 14), right=True)
    v2.logo(d, b, 24, name="Logo")
    n = cell("Nav", (0, 6, 0, 6), right=True)
    for lab in ("Home", "Files"):
        d.inst("Nav cell", n, props={"Label": lab, "State": "On" if lab == "Home" else "Off"}, name=f"Nav {lab}")
    apps_cell(d, n, compact=True)
    d.inst("Nav cell", n, props={"Label": "Admin", "State": "Off"}, name="Nav Admin")
    d.frame(hd, "Spacer", 0, 0, 4, 48, fill=None, sizingH="fill")
    u = cell("User", (0, 10, 0, 10), left=True)
    v2.avatar(d, u, OWNER[0])
    so = cell("Sign out", (0, 0, 0, 0), left=True, fixed=44)
    glyph(d, so, v2.ICONS["logout"], "muted", "Sign out glyph")
    ft = d.frame(f, "Footer", 0, PHONE_H - 34, PHONE_W, 34, fill="bg", layout="horizontal", padding=[0, 16, 0, 16],
                 mainAlign="max", crossAlign="center")
    side_stroke(d, ft, top=1)
    d.text(ft, "Look based on ArtCraft Launcher by the ArtCraft team", "Small/12", "muted", name="Credit")
    # The page behind: Home's hero, narrowed.
    top = vstack(d, f, "Hero", 16, 72, PHONE_W - 32, gap=12)
    hud(d, top, "01 / Home", color="muted", name="Kicker")
    v2.headline(d, top, "Your *cloud*,\nready.", style="Display/Hero 40", light=light)
    d.text(top, "Files live on this server and in a CraftCloud folder on each computer.", "Body/14", "muted",
           name="Body", w=PHONE_W - 32)
    apps_menu(d, f, 8, 48 + 4, w=PHONE_W - 16)
    return f


def build(d):
    """Makes the page "v2 · Apps" and its frames; returns [(export name, frame id)] like build_v2."""
    d.page(PAGE)
    frames = []
    col = W + GAP
    items = [("v2-web-apps-menu", lambda n, x, y: menu_frame(d, n, x, y)),
             ("v2-web-apps-menu-light", lambda n, x, y: menu_frame(d, n, x, y, light=True)),
             ("v2-web-apps-menu-phone", lambda n, x, y: phone_frame(d, n, x, y))]
    for c, (n, fn) in enumerate(items):
        frames.append((n, fn(n, c * col, 0)))
    return frames
