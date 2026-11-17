"""CraftCloud after the 2026-11-13 scope change: the simplified web frontend (Rust/egui → wasm) and the
desktop sync client (egui). Everything here is drawable in egui: hard edges, 1 px hairlines, flat fills,
one drop shadow for menus and dialogs, no blur, gradients or rounded corners."""

import os

from lib import APP, ASSETS, ORDER, USER, STORAGE, VERSION, OXICLOUD, KIND
from components import ICON
from screens import (W, H, GAP, DETAILS_W, hud, vstack, hstack, side_stroke, menu, scrim, dialog, dialog_actions,
                     doc_preview, screen, launcher_grid)

SERVER = "cloud.example.org"
URL = "https://cloud.example.org"
DW, DH = 960, 640  # desktop window

WEB_FILES = [
    # name, colour app (None: neutral), type, modified, size, shared, favourite
    ("Drafts", "folder", "Folder", "Today 09:10", "6 items", "—", False),
    ("Budget 2027.xlsx", "gridcraft", KIND["gridcraft"], "Yesterday 16:05", "86 KB", "2 people", True),
    ("Contract.pdf", "pdfcraft", "PDF document", "Today 08:30", "1.2 MB", "Link", False),
    ("Floorplan.dwg", "cadcraft", "DWG drawing", "11 Nov 2026", "3.4 MB", "—", False),
    ("IMG_2041.CR3", "lightcraft", "Canon raw photo", "9 Nov 2026", "28.6 MB", "—", False),
    ("Logo.svg", "vectorcraft", "SVG image", "Today 10:02", "14 KB", "Link", False),
    ("Notes.md", "wordcraft", "Markdown text", "Today 07:55", "3 KB", "—", False),
    ("Pitch.pptx", "deckcraft", KIND["deckcraft"], "10 Nov 2026", "5.8 MB", "2 people", False),
    ("Poster.psd", "photocraft", KIND["photocraft"], "Today 11:20", "42.1 MB", "—", False),
    ("Q3 report.docx", "wordcraft", KIND["wordcraft"], "Today 11:42", "248 KB", "Link", True),
    ("Theme.wav", "soundcraft", "WAV audio", "9 Nov 2026", "31.0 MB", "—", False),
    ("Trailer.mp4", "filmcraft", "MP4 video", "6 Nov 2026", "412 MB", "2 people", False),
    ("archive.zip", None, "ZIP archive", "2 Nov 2026", "96.3 MB", "—", False),
]
WEB_COLS = [("Name", None), ("Type", 170), ("Modified", 150), ("Size", 96), ("Shared", 104), ("", 44)]
DEV_VERSIONS = [
    ("Current", "Today 11:42", "248 KB", "Studio (desktop)"),
    ("", "Today 10:15", "244 KB", "Studio (desktop)"),
    ("", "Yesterday 17:30", "231 KB", "Laptop (desktop)"),
    ("", "Yesterday 09:12", "229 KB", "Web upload"),
    ("", "11 Nov 2026 15:48", "210 KB", "Laptop (desktop)"),
    ("", "10 Nov 2026 11:03", "188 KB", "Web upload"),
]


# --- components -------------------------------------------------------------------------------------

def new_components(d, lib):
    # Checkbox (selective sync tree).
    y = lib.row(14)
    ids = []
    for i, st in enumerate(("Off", "On", "Mixed")):
        on = st != "Off"
        c = d.comp(f"State={st}", i * 60, y, 14, 14, fill="invert_bg" if on else "bg",
                   stroke=None if on else "line_strong", clip=False)
        if st == "On":
            ck = d.inst("Icon/Check", c, 1, 1, name="Tick")
            d.set(d.sub(ck, "glyph"), stroke="invert_fg")
        if st == "Mixed":
            d.rect(c, 3, 6, 8, 2, "invert_fg", name="Dash")
        ids.append(c)
    s = d.R("component.combineVariants", {"ids": ids})["id"]
    d.set(s, name="Checkbox", x=0, y=y)
    d.C["Checkbox"] = s

    # File row for the web frontend: a Type column instead of "Opens in", a favourite star.
    y = lib.row(48)
    wlist = W - DETAILS_W - 48
    ids = []
    for i, state in enumerate(("Default", "Hover", "Selected")):
        c = d.comp(f"State={state}", 0, y + i * 70, wlist, 48,
                   fill={"Default": "bg", "Hover": "sunken", "Selected": "sunken"}[state],
                   layout="horizontal", crossAlign="center", sizingV="fixed")
        side_stroke(d, c, bottom=1)
        if state == "Selected":
            bar = d.rect(c, 0, 0, 2, 48, "accent_ink", name="Selected rule")
            d.absolute(bar, 0, 0)
        nc = d.frame(c, "Name cell", 0, 0, 300, 48, fill=None, layout="horizontal", padding=[0, 12, 0, 16], gap=12,
                     crossAlign="center", sizingH="fill", sizingV="fill")
        fi = d.inst("Icon/File", nc, name="File icon")
        d.set(d.sub(fi, "page"), fill="app:wordcraft/color")
        fo = d.inst("Icon/Folder", nc, name="Folder icon")
        d.set(fo, visible=False)
        nm = d.text(nc, "Q3 report.docx", "Body/14 Medium" if state == "Selected" else "Body/14", "ink",
                    name="File name")
        st_ = d.inst("Icon/Star", nc, name="Favourite star")
        d.set(st_, width=12, height=12)
        cells = {}
        for lab, w in WEB_COLS[1:]:
            cells[lab] = d.frame(c, f"{lab or 'More'} cell", 0, 0, w, 48, fill=None, layout="horizontal",
                                 padding=[0, 12, 0, 12] if lab else [0, 0, 0, 0], gap=8, crossAlign="center",
                                 mainAlign="center" if not lab else "min", sizingH="fixed", sizingV="fill")
        ty = d.text(cells["Type"], KIND["wordcraft"], "Body/13", "muted", name="Type")
        md = d.text(cells["Modified"], "Today 11:42", "Body/13", "muted", name="Modified")
        sz = d.text(cells["Size"], "248 KB", "Mono/12", "muted", name="Size")
        sh = d.text(cells["Shared"], "Link", "Body/13", "muted", name="Shared")
        d.text(cells[""], "…", "Body/14 Medium", "muted" if state == "Default" else "ink", name="More")
        for lay, pn in ((nm, "Name"), (ty, "Type"), (md, "Modified"), (sz, "Size"), (sh, "Shared")):
            d.prop(c, lay, pn)
        d.R("component.addProperty", {"id": c, "name": "Favourite", "type": "boolean", "layer": st_, "default": False})
        d.set(st_, visible=False)
        ids.append(c)
    s = d.R("component.combineVariants", {"ids": ids})["id"]
    d.set(s, name="File row/web", x=0, y=y)
    d.C["File row/web"] = s
    lib.y += 140

    # Desktop client title bar (960) and status bar.
    y = lib.row(48)
    h = d.comp("Desktop/Title bar", 0, y, DW, 48, fill="bg", layout="horizontal", crossAlign="center")
    side_stroke(d, h, bottom=1)

    def cell(name, pad=(0, 16, 0, 16), left=False, right=False, gap=8):
        c = d.frame(h, name, 0, 0, 10, 48, fill=None, layout="horizontal", padding=list(pad), gap=gap,
                    crossAlign="center", sizingH="hug", sizingV="fill", clip=False)
        if left or right:
            side_stroke(d, c, left=1 if left else 0, right=1 if right else 0)
        return c

    b = cell("Brand", (0, 20, 0, 20), right=True)
    d.inst("Brand/CraftCloud wordmark", b, name="Wordmark")
    t = cell("Tabs", (0, 12, 0, 12), gap=4)
    for lab in ("Activity", "Folders", "Settings"):
        d.inst("Nav cell", t, props={"Label": lab, "State": "On" if lab == "Activity" else "Off"}, name=f"Tab {lab}")
    d.frame(h, "Spacer", 0, 0, 10, 48, fill=None, sizingH="fill")
    sc = cell("Status", left=True)
    d.inst("Status badge", sc, props={"Tone": "Good", "Label": "Up to date"}, name="Sync badge")
    u = cell("Account", left=True, gap=10)
    av = d.frame(u, "Avatar", 0, 0, 26, 26, fill="sunken", stroke="line_strong", layout="horizontal",
                 mainAlign="center", crossAlign="center")
    d.text(av, USER[0], "Badge/10", "ink", name="Initials")
    col = vstack(d, u, "Who", 0, 0, 120, gap=1)
    d.text(col, USER[1], "Body/13", "ink", name="Account name")
    d.text(col, SERVER, "Small/12", "muted", name="Account server")

    y = lib.row(34)
    f = d.comp("Desktop/Status bar", 0, y, DW, 34, fill="bg", layout="horizontal", padding=[0, 20, 0, 20],
               crossAlign="center")
    side_stroke(d, f, top=1)
    lt = d.text(f, f"CraftCloud {VERSION} · ~/CraftCloud", "HUD/11", "muted", name="Left")
    d.frame(f, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    rt = d.text(f, "Last sync 14:32", "HUD/11", "muted", name="Right")
    d.prop(f, lt, "Left")
    d.prop(f, rt, "Right")


# --- helpers ----------------------------------------------------------------------------------------------

def onoff(d, parent, on, labels=("Off", "On"), name="Toggle", disabled=False):
    box = d.frame(parent, name, 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed",
                  clip=False)
    for i, lab in enumerate(labels):
        if i:
            d.rect(box, 0, 0, 1, 34, "line", name="sep")
        sel = (i == 1) == on if len(labels) == 2 else lab == on
        d.inst("Cell", box, props={"Label": lab, "Show count": False, "State": "On" if sel else "Off"},
               name=f"{name} {lab}")
    if disabled:
        d.set(box, opacity=0.5)
    return box


def field(d, parent, label, value, state="Default", w=None, hint=None, hint_color="muted", button=None, suffix=None):
    fl = vstack(d, parent, f"Field {label}", 0, 0, w or 300, gap=8)
    d.set(fl, sizingH="fill")
    hud(d, fl, label, color="ink", name=f"{label} label")
    r = hstack(d, fl, f"{label} row", gap=8)
    d.set(r, sizingH="fill")
    i = d.inst("Input", r, props={"State": state, "Value": value}, name=f"{label} input")
    d.set(i, sizingH="fill")
    if suffix:
        d.text(r, suffix, "Mono/13", "muted", name="Suffix")
    if button:
        d.inst("Button", r, props={"Kind": "Secondary", "Label": button}, name=button)
    if hint:
        d.text(fl, hint, "Small/12.5", hint_color, name=f"{label} hint", w=w or 300)
        d.set(d.sub(fl, f"{label} hint"), sizingH="fill")
    return fl


def panel(d, parent, title, w, pad=(16, 18, 18, 18), gap=12, admin=False):
    p = vstack(d, parent, f"Panel {title}", 0, 0, w, gap=gap, pad=list(pad))
    d.set(p, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [p], "field": "strokes/0/color", "variable": "line"})
    r = hstack(d, p, "Panel head", gap=10)
    hud(d, r, title, color="ink", name="Panel title")
    if admin:
        d.inst("Status badge", r, props={"Tone": "Accent", "Label": "Admin only"}, name="Admin badge")
    return p


def kv(d, parent, k, v, kw=150, mono=False, color="ink"):
    r = hstack(d, parent, k, gap=0)
    lab = hud(d, r, k, name=f"{k} key")
    d.set(lab, sizingH="fixed", width=kw)
    d.text(r, v, "Mono/13" if mono else "Body/13", color, name=f"{k} value")
    return r


def buttons(d, parent, spec, name="Buttons", align="max", w=None):
    r = hstack(d, parent, name, gap=8, mainAlign=align)
    if w:
        d.set(r, sizingH="fill")
    out = []
    for kind, label in spec:
        out.append(d.inst("Button", r, props={"Kind": kind, "Label": label}, name=label))
    return r, out


def web_page(d, name, x, y, section="Files", light=False, apps=False, footer_left=None):
    f = screen(d, name, x, y, light=light)
    comp = "Header/Web with apps" if apps else "Header/Web"
    hid = d.inst(comp, f, 0, 0, name="Header")
    navs = ("Apps", "Files", "Recent", "Favourites", "Shared", "Trash", "Admin") if apps else \
        ("Files", "Recent", "Favourites", "Shared", "Trash", "Admin")
    for lab in navs:
        d.R("instance.setProperties", {"ids": [d.sub(hid, f"Nav {lab}")],
                                       "properties": {"State": "On" if lab == section else "Off"}})
    ft = d.inst("Footer", f, 0, H - 34, name="Footer")
    d.R("instance.setProperties", {"ids": [ft], "properties": {
        "Left": footer_left or f"CraftCloud {VERSION} · {SERVER}"}})
    return f, hid


def page_title(d, f, title, kicker=None, x=24, y=72):
    col = vstack(d, f, "Title block", x, y, 800, gap=6)
    if kicker:
        hud(d, col, kicker, name="Kicker")
    d.text(col, title, "Display/Page title 32", "ink_strong", name="Page title")
    return col


def web_row(d, parent, x, y, fl, state="Default"):
    name, app, typ, mod, size, shared, fav = fl
    iid = d.inst("File row/web", parent, x, y, name=f"Row {name}")
    d.R("instance.setProperties", {"ids": [iid], "properties": {
        "State": state, "Name": name, "Type": typ, "Modified": mod, "Size": size, "Shared": shared, "Favourite": fav}})
    if app in KIND and typ == KIND[app]:
        d.R("variable.bind", {"ids": [iid], "field": "componentProperties/Type", "variable": f"{app}/kind"})
    if app == "folder":
        d.set(d.sub(iid, "File icon"), visible=False)
        d.set(d.sub(iid, "Folder icon"), visible=True)
    elif app is None:
        d.set(d.sub(d.sub(iid, "File icon"), "page"), fill="muted")
    else:
        d.set(d.sub(d.sub(iid, "File icon"), "page"), fill=f"app:{app}/color")
    return iid


def list_header(d, parent, w, cols, sort="Name"):
    hd = d.frame(parent, "List header", 0, 0, w, 40, fill=None, layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1, top=1)
    for lab, cw in cols:
        c = d.frame(hd, f"Head {lab or 'more'}", 0, 0, cw or 100, 40, fill=None, layout="horizontal",
                    padding=[0, 12, 0, 16 if lab == cols[0][0] else 12], gap=6, crossAlign="center",
                    sizingH="fill" if cw is None else "fixed", sizingV="fill")
        if lab:
            hud(d, c, lab, color="ink" if lab == sort else "muted", name=f"{lab} label")
        if lab == sort:
            d.inst("Icon/Sort up", c, name="Sort arrow")
    return hd


def generic_row(d, parent, w, cells, h=48, state="Default", name="Row"):
    """A table row of plain cells: cells = [(width|None, builder(cell))]."""
    r = d.frame(parent, name, 0, 0, w, h, fill="sunken" if state != "Default" else "bg", layout="horizontal",
                crossAlign="center", sizingV="fixed")
    side_stroke(d, r, bottom=1)
    for i, (cw, build) in enumerate(cells):
        c = d.frame(r, f"Cell {i}", 0, 0, cw or 100, h, fill=None, layout="horizontal",
                    padding=[0, 12, 0, 16 if i == 0 else 12], gap=8, crossAlign="center",
                    sizingH="fill" if cw is None else "fixed", sizingV="fill")
        build(c)
    return r


def file_icon(d, parent, app):
    if app == "folder":
        return d.inst("Icon/Folder", parent, name="Folder icon")
    fi = d.inst("Icon/File", parent, name="File icon")
    d.set(d.sub(fi, "page"), fill=f"app:{app}/color" if app else "muted")
    return fi


# --- 1. setup wizard ----------------------------------------------------------------------------------

STEPS = ("Welcome", "Admin", "Server", "Web apps", "Done")


def setup_frame(d, name, x, y, step):
    f = screen(d, name, x, y)
    hd = d.frame(f, "Setup header", 0, 0, W, 48, fill="bg", layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1)
    b = d.frame(hd, "Brand", 0, 0, 10, 48, fill=None, layout="horizontal", padding=[0, 20, 0, 20],
                crossAlign="center", sizingH="hug", sizingV="fill")
    side_stroke(d, b, right=1)
    d.inst("Brand/CraftCloud wordmark", b, name="Wordmark")
    s = d.frame(hd, "Suite", 0, 0, 10, 48, fill=None, layout="horizontal", padding=[0, 16, 0, 16],
                crossAlign="center", sizingH="hug", sizingV="fill")
    side_stroke(d, s, right=1)
    hud(d, s, "First-run setup", name="Setup label")
    d.frame(hd, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    r = d.frame(hd, "Step count", 0, 0, 10, 48, fill=None, layout="horizontal", padding=[0, 20, 0, 20],
                crossAlign="center", sizingH="hug", sizingV="fill")
    side_stroke(d, r, left=1)
    hud(d, r, f"Step {step} of 5", name="Step count label")
    # Step indicator: HUD cells in one hairline box; done steps carry a tick in good.
    cw = 720
    x0 = (W - cw) / 2
    ind = d.frame(f, "Step indicator", x0, 88, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                  sizingV="fixed", clip=False)
    for i, lab in enumerate(STEPS, 1):
        if i > 1:
            d.rect(ind, 0, 0, 1, 34, "line", name="sep")
        st = "On" if i == step else "Off"
        c = d.inst("Cell", ind, props={"Label": f"{i:02d} {lab}", "Show count": i < step, "Count": "✓" if False else "",
                                         "State": st}, name=f"Step {lab}")
        if i < step:
            d.set(d.sub(c, "Label"), fill="good")
            d.R("instance.setProperties", {"ids": [c], "properties": {"Show count": False}})
    body = vstack(d, f, "Setup body", x0, 160, cw, gap=22)
    ft = d.frame(f, "Setup footer", 0, H - 34, W, 34, fill="bg", layout="horizontal", padding=[0, 24, 0, 24],
                 crossAlign="center")
    side_stroke(d, ft, top=1)
    hud(d, ft, f"CraftCloud {VERSION} · OxiCloud {OXICLOUD}", name="Footer left")
    d.frame(ft, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.text(ft, "Look based on ArtCraft Launcher by the ArtCraft team", "Small/12", "muted", name="Attribution")
    return f, body, cw


def setup_actions(d, body, cw, back=True, primary="Continue"):
    r = d.frame(body, "Setup actions", 0, 0, cw, 66, fill=None, layout="horizontal", gap=8,
                padding=[18, 0, 0, 0], crossAlign="center", sizingH="fill", sizingV="hug")
    side_stroke(d, r, top=1)
    if back:
        d.inst("Button", r, props={"Kind": "Secondary", "Label": "Back"}, name="Back")
    d.frame(r, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Primary", "Label": primary}, name=primary)
    return r


def setup_1(d, name, x, y):
    f, body, cw = setup_frame(d, name, x, y, 1)
    hud(d, body, "Welcome", name="Kicker")
    d.text(body, "Your own cloud for files", "Display/Page title 32", "ink_strong", name="Title")
    d.text(body, "CraftCloud keeps your files on this server. The desktop client syncs a folder on each computer, "
                 "so the Craft apps open and save files there like any other folder. This setup takes a minute: "
                 "an admin account, the server's address, and whether to host the Craft apps in the browser.",
           "Body/14", "ink", name="Intro", w=cw)
    p = panel(d, body, "This server", cw)
    kv(d, p, "Image", f"craftcloud:{VERSION} (slim)", mono=True)
    kv(d, p, "Backend", f"OxiCloud {OXICLOUD}", mono=True)
    kv(d, p, "Data", "/data · Docker volume craftcloud-data", mono=True)
    d.text(body, "Setup is reachable only until the admin account exists. After that this page sends people to "
                 "sign in.", "Small/12.5", "muted", name="Note", w=cw)
    setup_actions(d, body, cw, back=False, primary="Start setup")
    return f


def setup_2(d, name, x, y):
    f, body, cw = setup_frame(d, name, x, y, 2)
    hud(d, body, "Admin account", name="Kicker")
    d.text(body, "Create the admin account", "Display/Page title 32", "ink_strong", name="Title")
    d.text(body, "The admin can add people, see storage and change server settings. You can add more admins later.",
           "Body/14", "muted", name="Intro", w=cw)
    field(d, body, "Name", USER[1], w=cw)
    field(d, body, "Username", "john", w=cw, hint="Used to sign in and in share links. Lower case, no spaces.")
    field(d, body, "Password", "••••••••••••••••", w=cw)
    field(d, body, "Repeat password", "••••••••••••••••", state="Focus", w=cw,
          hint="Strong · 16 characters · passwords match", hint_color="good")
    setup_actions(d, body, cw)
    return f


def setup_3(d, name, x, y):
    f, body, cw = setup_frame(d, name, x, y, 3)
    hud(d, body, "Server", name="Kicker")
    d.text(body, "Where people reach this server", "Display/Page title 32", "ink_strong", name="Title")
    fl = field(d, body, "Public server URL", URL, w=cw, button="Check",
               hint="Filled in from the address you opened this page with. Desktop clients and share links use it.")
    res = hstack(d, body, "Check result", gap=10)
    d.inst("Status badge", res, props={"Tone": "Good", "Label": "Reachable"}, name="Result badge")
    d.text(res, f"{URL} answered in 84 ms · TLS certificate valid until 12 Feb 2027 · WebDAV OK", "Small/12.5",
           "ink", name="Result text")
    p = panel(d, body, "Storage", cw)
    kv(d, p, "Location", "/data (Docker volume craftcloud-data)", mono=True)
    kv(d, p, "Free", "812 GB of 1 TB")
    kv(d, p, "Versions", "Before any overwrite, the old file goes to .craftcloud/versions/ · last 5 kept")
    d.text(p, "To move the files, stop the container and mount another volume at /data. Setup can't change it.",
           "Small/12.5", "muted", name="Storage note", w=cw - 36)
    del fl
    setup_actions(d, body, cw)
    return f


def setup_4(d, name, x, y):
    f, body, cw = setup_frame(d, name, x, y, 4)
    hud(d, body, "Web apps · optional", name="Kicker")
    d.text(body, "Craft apps in the browser", "Display/Page title 32", "ink_strong", name="Title")
    d.text(body, "Off by default. With web apps on, this server also hosts the 15 Craft apps, so files open in the "
                 "browser without installing anything. Most people use the desktop client and the desktop apps "
                 "instead.", "Body/14", "ink", name="Intro", w=cw)
    r = hstack(d, body, "Toggle row", gap=16)
    onoff(d, r, False, name="Web apps", disabled=True)
    d.text(r, "Web apps are off · Needs the -apps image", "Body/14 Medium", "ink", name="Toggle state")
    p = panel(d, body, "What it needs", cw)
    kv(d, p, "Image", f"craftcloud:{VERSION}-apps instead of craftcloud:{VERSION}", mono=True)
    kv(d, p, "Size", "the -apps image: about 770 MB download, 2.1 GB on disk")
    kv(d, p, "Setting", "CRAFTCLOUD_WEB_APPS=on seeds the first start; then Admin › Server", mono=True)
    icons = hstack(d, p, "App icons", gap=6)
    for a in ORDER:
        ic = d.inst("App icon/small", icons, name=f"Icon {a}")
        d.R("image.replace", {"ids": [d.sub(ic, "Icon image")], "path": ICON(a)})
    d.set(icons, opacity=0.5)
    note = hstack(d, body, "Image note", gap=10)
    d.inst("Status badge", note, props={"Tone": "Warn", "Label": "Slim image"}, name="Slim badge")
    d.text(note, f"This server runs craftcloud:{VERSION}, which has no web apps, so the toggle can't be turned on "
                 "here (Needs the -apps image).", "Small/12.5", "ink", name="Image note text")
    d.text(body, "You can change this later in Admin › Server. The setting is stored on the server in "
                 "/app/storage/.craftcloud/settings.env.", "Small/12.5", "muted", name="Later note", w=cw)
    setup_actions(d, body, cw)
    return f


def setup_5(d, name, x, y):
    f, body, cw = setup_frame(d, name, x, y, 5)
    hud(d, body, "Done", color="good", name="Kicker")
    d.text(body, "CraftCloud is ready", "Display/Page title 32", "ink_strong", name="Title")
    d.text(body, f"Signed in as {USER[1]} (admin). Next, put the desktop client on your computers.", "Body/14",
           "muted", name="Intro", w=cw)
    grid = d.frame(body, "Next steps", 0, 0, cw, 10, fill=None, layout="horizontal", gap=0, sizingH="fill",
                   sizingV="hug", crossAlign="min", clip=False)
    for title, text, kind, label, extra in (
            ("Desktop client", "Syncs a folder on this computer with the cloud. Linux build detected.", "Primary",
             "Download for Linux", "Windows and macOS builds →"),
            ("Files", "Upload, share and restore versions in the browser.", "Secondary", "Go to files", None)):
        c = vstack(d, grid, f"Card {title}", 0, 0, cw / 2, gap=10, pad=[20, 20, 20, 20])
        d.set(c, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
        d.R("variable.bind", {"ids": [c], "field": "strokes/0/color", "variable": "line"})
        d.text(c, title, "Display/Heading 24", "ink_strong", name="Card title")
        d.text(c, text, "Body/14", "muted", name="Card text", w=cw / 2 - 40)
        b = d.inst("Button", c, props={"Kind": kind, "Label": label}, name=label)
        d.set(b, sizingH="fill")
        if extra:
            d.text(c, extra, "Small/12.5", "accent_ink", name="Card link")
    p = panel(d, body, "Connect a device", cw)
    r = hstack(d, p, "URL row", gap=8)
    d.set(r, sizingH="fill")
    i = d.inst("Input", r, props={"Value": URL}, name="Server URL")
    d.set(i, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Copy"}, name="Copy")
    d.text(p, "Paste this address into the desktop client when it asks for a server.", "Small/12.5", "muted",
           name="Connect note")
    return f


# --- 2. web login ---------------------------------------------------------------------------------------

def web_login(d, name, x, y):
    f = screen(d, name, x, y)
    cw = 420
    card = vstack(d, f, "Sign-in card", (W - cw) / 2, 150, cw, gap=0, fill="bg")
    d.set(card, strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [card], "field": "strokes/0/color", "variable": "line"})
    top = vstack(d, card, "Brand block", 0, 0, cw, gap=10, pad=[36, 32, 28, 32])
    d.set(top, sizingH="fill")
    side_stroke(d, top, bottom=1)
    wm = d.inst("Brand/CraftCloud wordmark", top, name="Wordmark")
    d.R("transform.scale", {"ids": [wm], "factor": 1.6})
    hud(d, top, SERVER, name="Server")
    form = vstack(d, card, "Form", 0, 0, cw, gap=16, pad=[28, 32, 32, 32])
    d.set(form, sizingH="fill")
    field(d, form, "Username", "john", state="Focus", w=cw - 64)
    field(d, form, "Password", "••••••••••••", w=cw - 64)
    b = d.inst("Button", form, props={"Kind": "Primary", "Label": "Sign in"}, name="Sign in")
    d.set(b, sizingH="fill")
    dv = hstack(d, form, "Device note", gap=8)
    d.inst("Status badge", dv, props={"Tone": "Accent", "Label": "Desktop client"}, name="Device badge")
    d.text(dv, "Studio is waiting for this sign-in · code KQ7-4XM", "Small/12.5", "ink", name="Device text")
    d.text(form, "Forgot your password? Ask your admin to reset it.", "Small/12.5", "muted", name="Forgot")
    ft = d.frame(f, "Login footer", 0, H - 48, W, 48, fill=None, layout="horizontal", mainAlign="center",
                 crossAlign="center")
    d.text(ft, f"CraftCloud {VERSION} · {SERVER} · look based on ArtCraft Launcher by the ArtCraft team", "HUD/11",
           "muted", name="Footer line")
    return f


# --- 3. files -----------------------------------------------------------------------------------------------

def files_toolbar(d, f, wlist, crumbs=("Files", "Documents", "Reports")):
    tb = d.frame(f, "Files toolbar", 24, 72, wlist, 34, fill=None, layout="horizontal", gap=8, crossAlign="center",
                 clip=False)
    bc = d.frame(tb, "Breadcrumbs", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed")
    for i, part in enumerate(crumbs):
        if i:
            sep = d.frame(bc, "sep", 0, 0, 18, 34, fill=None, layout="horizontal", mainAlign="center",
                          crossAlign="center")
            side_stroke(d, sep, left=1, right=1)
            d.text(sep, "›", "Body/13", "muted", name="›")
        d.inst("Cell", bc, props={"Label": part, "Show count": False, "State": "On" if i == len(crumbs) - 1 else "Off"},
               name=f"Crumb {part}")
    d.frame(tb, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", tb, props={"Kind": "Primary", "Label": "Upload"}, name="Upload")
    d.inst("Button", tb, props={"Kind": "Secondary", "Label": "New folder"}, name="New folder")
    tg = d.frame(tb, "View toggle", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed")
    for lab, on in (("Grid", False), ("List", True)):
        c = d.frame(tg, lab, 0, 0, 34, 34, fill="invert_bg" if on else None, layout="horizontal",
                    mainAlign="center", crossAlign="center")
        ic = d.inst(f"Icon/{lab}", c, name=f"{lab} icon")
        if on:
            for k in d.R("node.find", {"root": ic, "type": "RECTANGLE"})["ids"]:
                d.set(k, fill="invert_fg")
    return tb


def web_details(d, f, x):
    p = d.frame(f, "Details", x, 48, DETAILS_W, H - 48 - 34, fill="bg", layout="vertical", padding=[24, 24, 24, 24],
                gap=16, clip=True)
    side_stroke(d, p, left=1)
    doc_preview(d, p, DETAILS_W - 48, 136)
    nm = hstack(d, p, "Name block", gap=10)
    d.set(nm, sizingH="fill")
    col = vstack(d, nm, "Name col", 0, 0, 250, gap=4)
    d.set(col, sizingH="fill")
    d.text(col, "Q3 report.docx", "Display/Dialog title 22", "ink_strong", name="Detail name")
    d.text(col, f"{KIND['wordcraft']} · 248 KB", "Small/12.5", "muted", name="Detail kind")
    fav = d.frame(nm, "Favourite", 0, 0, 34, 34, fill=None, stroke="line_strong", layout="horizontal",
                  mainAlign="center", crossAlign="center")
    d.inst("Icon/Star", fav, name="Star")
    ac = hstack(d, p, "Detail actions", gap=8)
    d.set(ac, sizingH="fill")
    o = d.inst("Button", ac, props={"Kind": "Primary", "Label": "Download"}, name="Download")
    d.set(o, sizingH="fill")
    s = d.inst("Button", ac, props={"Kind": "Secondary", "Label": "Share…"}, name="Share")
    d.set(s, sizingH="fill")
    d.inst("Button/More", ac, name="More")
    facts = vstack(d, p, "Facts", 0, 0, DETAILS_W - 48, gap=6)
    for k, v in (("Modified", "Today 11:42 on Studio"), ("Owner", USER[1]), ("Path", "/Documents/Reports"),
                 ("Synced to", "Studio, Laptop")):
        r = hstack(d, facts, f"Fact {k}", gap=0)
        lab = hud(d, r, k, name=f"{k} key")
        d.set(lab, sizingH="fixed", width=96)
        d.text(r, v, "Mono/12" if k == "Path" else "Body/13", "ink", name=f"{k} value")
    vs = vstack(d, p, "Versions", 0, 0, DETAILS_W - 48, gap=0)
    vh = hstack(d, vs, "Versions head", gap=8, padding=[0, 0, 8, 0])
    d.set(vh, sizingH="fill")
    hud(d, vh, "Versions", color="ink", name="Versions title")
    d.frame(vh, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    hud(d, vh, "Last 5 kept", name="Versions note")
    for i, (cur, when, size, by) in enumerate(DEV_VERSIONS):
        r = d.frame(vs, f"Version {i}", 0, 0, DETAILS_W - 48, 30, fill=None, layout="horizontal", gap=8,
                    crossAlign="center", sizingH="fill", sizingV="fixed")
        side_stroke(d, r, top=1)
        d.text(r, when, "Body/13 Medium" if cur else "Body/13", "ink", name="When")
        d.text(r, by.split(" ")[0], "Small/12", "muted", name="By")
        d.frame(r, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
        if cur:
            d.inst("Status badge", r, props={"Tone": "Good", "Label": "Current"}, name="Current")
        else:
            d.text(r, "Restore", "Button/11", "accent_ink", name="Restore")
    sl = vstack(d, p, "Share link", 0, 0, DETAILS_W - 48, gap=8)
    sh = hstack(d, sl, "Share head", gap=8)
    d.set(sh, sizingH="fill")
    hud(d, sh, "Share link", color="ink", name="Share title")
    d.frame(sh, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Status badge", sh, props={"Tone": "Good", "Label": "On · view only"}, name="Link state")
    r = hstack(d, sl, "Link row", gap=8)
    d.set(r, sizingH="fill")
    li = d.inst("Input", r, props={"Value": f"{SERVER}/s/7Kq2xW"}, name="Link")
    d.set(li, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Copy"}, name="Copy")
    return p


def web_files(d, name, x, y, light=False):
    f, _ = web_page(d, name, x, y, "Files", light=light,
                    footer_left=f"{STORAGE[0]} used · versions kept: 5 · {SERVER}")
    wlist = W - DETAILS_W - 48
    files_toolbar(d, f, wlist)
    lst = d.frame(f, "File list", 24, 130, wlist, 40 + 48 * len(WEB_FILES), fill=None, clip=False)
    list_header(d, lst, wlist, WEB_COLS)
    for i, fl in enumerate(WEB_FILES):
        st = "Selected" if fl[0] == "Q3 report.docx" else ("Hover" if fl[0] == "Logo.svg" else "Default")
        web_row(d, lst, 0, 40 + i * 48, fl, st)
    yb = 130 + 40 + 48 * len(WEB_FILES) + 10
    drop = d.frame(f, "Drop hint", 24, yb, wlist, 40, fill=None, stroke="line", layout="horizontal", gap=10,
                   padding=[0, 16, 0, 16], crossAlign="center")
    hud(d, drop, "Drop files here to upload them to Reports", name="Drop label")
    d.frame(drop, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.text(drop, "13 items · 625 MB · synced to 2 devices", "Small/12", "muted", name="List summary")
    web_details(d, f, W - DETAILS_W)
    return f


def web_search(d, name, x, y):
    f, hid = web_page(d, name, x, y, "Files")
    s = d.find(hid, "Search")[-1]  # the cell and the field share the name; the field is the inner one
    d.R("instance.setProperties", {"ids": [s], "properties": {"Placeholder": "report"}})
    d.set(d.sub(s, "Placeholder"), fill="ink")
    d.set(s, strokes=[{"type": "solid", "color": "#a9c48e"}], strokeWeight=2)
    d.R("variable.bind", {"ids": [s], "field": "strokes/0/color", "variable": "focus"})
    page_title(d, f, "Results for “report”", kicker="Search · 6 results in all folders")
    filt = d.frame(f, "Filters", 24, 160, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                   sizingV="fixed", clip=False)
    for i, (lab, n) in enumerate((("All", 6), ("Documents", 4), ("Spreadsheets", 1), ("Folders", 1))):
        if i:
            d.rect(filt, 0, 0, 1, 34, "line", name="sep")
        d.inst("Cell", filt, props={"Label": lab, "Count": str(n), "State": "On" if i == 0 else "Off"},
               name=f"Filter {lab}")
    lw = W - 48
    cols = [("Name", None), ("Folder", 320), ("Modified", 170), ("Size", 110), ("", 44)]
    lst = vstack(d, f, "Results", 24, 214, lw, gap=0)
    list_header(d, lst, lw, cols, sort="Name")
    rows = [("Q3 report.docx", "wordcraft", "/Documents/Reports", "Today 11:42", "248 KB"),
            ("Q2 report.docx", "wordcraft", "/Documents/Reports/2026", "2 Jul 2026", "231 KB"),
            ("Annual report 2025.pdf", "pdfcraft", "/Documents/Archive", "14 Mar 2026", "4.6 MB"),
            ("Expense report.xlsx", "gridcraft", "/Finance", "31 Oct 2026", "64 KB"),
            ("Report template.dotx", "wordcraft", "/Templates", "8 Jan 2026", "52 KB"),
            ("report-photos", "folder", "/Photos/2026", "Yesterday 18:20", "48 items")]
    for i, (nm, app, folder, mod, size) in enumerate(rows):
        def b0(c, nm=nm, app=app):
            file_icon(d, c, app)
            t = d.text(c, nm, "Body/14", "ink", name="Name")
            k = nm.lower().find("report")
            if k >= 0:
                d.R("text.style", {"id": t, "start": k, "end": k + 6, "textFills": ["#a9c48e"]})
        generic_row(d, lst, lw, [
            (None, b0),
            (320, lambda c, folder=folder: d.text(c, folder, "Mono/12", "muted", name="Folder")),
            (170, lambda c, mod=mod: d.text(c, mod, "Body/13", "muted", name="Modified")),
            (110, lambda c, size=size: d.text(c, size, "Mono/12", "muted", name="Size")),
            (44, lambda c: d.text(c, "…", "Body/14 Medium", "muted", name="More"))],
            state="Hover" if i == 0 else "Default", name=f"Result {nm}")
    d.text(f, "Search looks at names in all your folders and the folders shared with you. Matches are in accent ink.",
           "Small/12.5", "muted", x=24, y=214 + 40 + 6 * 48 + 16, name="Search note")
    return f


def web_recent(d, name, x, y):
    f, _ = web_page(d, name, x, y, "Recent")
    page_title(d, f, "Recent", kicker="Files you and your devices changed")
    lw = W - 48
    cols = [("Name", None), ("Folder", 280), ("Changed on", 220), ("When", 150), ("", 44)]
    lst = vstack(d, f, "Recent list", 24, 160, lw, gap=0)
    list_header(d, lst, lw, cols, sort="When")
    groups = [("Today", [("Q3 report.docx", "wordcraft", "/Documents/Reports", "Studio (desktop)", "11:42"),
                         ("Poster.psd", "photocraft", "/Design", "Laptop (desktop)", "11:20"),
                         ("Logo.svg", "vectorcraft", "/Design", "Web upload", "10:02"),
                         ("Contract.pdf", "pdfcraft", "/Documents", "Studio (desktop)", "08:30")]),
              ("Yesterday", [("Budget 2027.xlsx", "gridcraft", "/Finance", "Laptop (desktop)", "16:05"),
                             ("report-photos", "folder", "/Photos/2026", "Web upload", "18:20")]),
              ("This week", [("Floorplan.dwg", "cadcraft", "/Projects/Studio", "Studio (desktop)", "Wed 11 Nov"),
                             ("Pitch.pptx", "deckcraft", "/Documents", "Laptop (desktop)", "Tue 10 Nov")])]
    for g, items in groups:
        gh = d.frame(lst, f"Group {g}", 0, 0, lw, 36, fill=None, layout="horizontal", padding=[0, 16, 0, 16],
                     crossAlign="center", sizingH="fill")
        side_stroke(d, gh, bottom=1)
        hud(d, gh, g, color="ink", name="Group label")
        for nm, app, folder, dev, when in items:
            def b0(c, nm=nm, app=app):
                file_icon(d, c, app)
                d.text(c, nm, "Body/14", "ink", name="Name")

            def b2(c, dev=dev):
                d.text(c, dev, "Body/13", "muted" if dev == "Web upload" else "ink", name="Device")
            generic_row(d, lst, lw, [
                (None, b0), (280, lambda c, folder=folder: d.text(c, folder, "Mono/12", "muted", name="Folder")),
                (220, b2), (150, lambda c, when=when: d.text(c, when, "Body/13", "muted", name="When")),
                (44, lambda c: d.text(c, "…", "Body/14 Medium", "muted", name="More"))], h=44, name=f"Recent {nm}")
    return f


def web_trash(d, name, x, y):
    f, _ = web_page(d, name, x, y, "Trash")
    page_title(d, f, "Trash", kicker="7 items · 1.2 GB")
    r = hstack(d, f, "Trash actions", 0, 88, gap=8)
    d.inst("Button", r, props={"Kind": "Danger", "Label": "Empty trash"}, name="Empty trash")
    d.set(r, x=W - 24 - d.R("node.get", {"id": r, "fields": ["width"]})["width"])
    lw = W - 48
    cols = [("Name", None), ("Original location", 280), ("Deleted", 170), ("Size", 100), ("", 300)]
    lst = vstack(d, f, "Trash list", 24, 160, lw, gap=0)
    list_header(d, lst, lw, cols, sort="Deleted")
    items = [("Old budget.xlsx", "gridcraft", "/Finance", "Today 09:12", "71 KB"),
             ("Moodboard v1.psd", "photocraft", "/Design", "Yesterday 15:40", "88.2 MB"),
             ("Interview.wav", "soundcraft", "/Audio", "10 Nov 2026", "640 MB"),
             ("Drafts 2025", "folder", "/Documents", "8 Nov 2026", "41 items"),
             ("Q3 report (conflict 2026-11-12 09.03 Laptop).docx", "wordcraft", "/Documents/Reports", "7 Nov 2026",
              "240 KB"),
             ("logo-old.svg", "vectorcraft", "/Design", "2 Nov 2026", "12 KB"),
             ("scan_0042.pdf", "pdfcraft", "/Inbox", "28 Oct 2026", "3.1 MB")]
    for i, (nm, app, loc, when, size) in enumerate(items):
        def b0(c, nm=nm, app=app):
            file_icon(d, c, app)
            d.text(c, nm, "Body/14", "ink", name="Name")

        def b4(c, i=i):
            d.set(c, mainAlign="max")
            if i == 1:
                d.inst("Button", c, props={"Kind": "Secondary", "Label": "Restore"}, name="Restore")
                d.inst("Button", c, props={"Kind": "Danger", "Label": "Delete forever"}, name="Delete forever")
            else:
                d.text(c, "Restore", "Button/11", "accent_ink", name="Restore")
        generic_row(d, lst, lw, [
            (None, b0), (280, lambda c, loc=loc: d.text(c, loc, "Mono/12", "muted", name="Location")),
            (170, lambda c, when=when: d.text(c, when, "Body/13", "muted", name="Deleted")),
            (100, lambda c, size=size: d.text(c, size, "Mono/12", "muted", name="Size")), (300, b4)],
            state="Hover" if i == 1 else "Default", name=f"Trash {nm}")
    d.text(f, "Items in the trash are deleted for good after 30 days. Restoring puts an item back where it was; if "
              "that folder is gone, it goes to Files.", "Small/12.5", "muted", x=24, y=160 + 40 + 7 * 48 + 16,
           name="Trash note")
    return f


# --- 5/6. dialogs over the files view -------------------------------------------------------------------------

def share_dialog(d, name, x, y, backdrop):
    from screens import duplicate_into
    f = duplicate_into(d, backdrop, name, x, y)
    scrim(d, f)
    w = 560
    dl, body = dialog(d, f, (W - w) / 2, 140, w, "Share Q3 report.docx", hud_label="Documents › Reports")
    r = hstack(d, body, "Link toggle", gap=14)
    onoff(d, r, True, name="Link")
    d.text(r, "Anyone with the link can open it", "Body/14", "ink", name="Link state")
    fl = field(d, body, "Link", f"https://{SERVER}/s/7Kq2xW", w=w - 48, button="Copy")
    del fl
    r = vstack(d, body, "Access", 0, 0, w - 48, gap=8)
    hud(d, r, "Access", color="ink", name="Access label")
    onoff(d, r, "View only", labels=("View only", "Can edit"), name="Access")
    r = vstack(d, body, "Expiry", 0, 0, w - 48, gap=8)
    hud(d, r, "Expires", color="ink", name="Expiry label")
    sel = d.inst("Select", r, props={"Value": "In 30 days · 13 Dec 2026"}, name="Expiry picker")
    d.set(sel, sizingH="fill")
    r = vstack(d, body, "Password", 0, 0, w - 48, gap=8)
    hr = hstack(d, r, "Password head", gap=12)
    hud(d, hr, "Password", color="ink", name="Password label")
    onoff(d, hr, True, name="Password toggle")
    i = d.inst("Input", r, props={"State": "Focus", "Value": "••••••••"}, name="Password input")
    d.set(i, sizingH="fill")
    d.text(body, "People with the link need this password. Send it separately.", "Small/12.5", "muted",
           name="Password note", w=w - 48)
    row = d.frame(dl, "Dialog actions", 0, 0, w, 66, fill=None, layout="horizontal", gap=8, padding=[16, 24, 16, 24],
                  crossAlign="center", sizingH="fill", sizingV="hug")
    side_stroke(d, row, top=1)
    d.inst("Button", row, props={"Kind": "Secondary", "Label": "Stop sharing"}, name="Stop sharing")
    d.frame(row, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", row, props={"Kind": "Primary", "Label": "Done"}, name="Done")
    return f


def versions_dialog(d, name, x, y, backdrop):
    from screens import duplicate_into
    f = duplicate_into(d, backdrop, name, x, y)
    scrim(d, f)
    w = 780
    dl, body = dialog(d, f, (W - w) / 2, 150, w, "Versions of Q3 report.docx", hud_label="Documents › Reports")
    d.text(body, "Before any overwrite, from a desktop client or the web, the old file is kept. CraftCloud keeps the "
                 "last 5. Restoring puts the current file into the list first, so nothing is lost.", "Body/14", "muted",
           name="Note", w=w - 48)
    tbl = d.frame(body, "Versions table", 0, 0, w - 48, 10, stroke="line", layout="vertical", sizingV="hug",
                  sizingH="fill")
    cols = (("Saved", 230), ("Size", 100), ("Saved from", 180), ("", 222))
    hd = hstack(d, tbl, "Head", gap=0, h=38)
    d.set(hd, sizingH="fill")
    side_stroke(d, hd, bottom=1)
    for lab, cw in cols:
        c = d.frame(hd, f"H {lab}", 0, 0, cw, 38, fill=None, layout="horizontal", padding=[0, 14, 0, 14],
                    crossAlign="center", sizingV="fill")
        if lab:
            hud(d, c, lab, name=f"{lab} label")
    for i, (cur, when, size, by) in enumerate(DEV_VERSIONS):
        r = hstack(d, tbl, f"Version {i}", gap=0, h=52)
        d.set(r, sizingH="fill")
        if i < len(DEV_VERSIONS) - 1:
            side_stroke(d, r, bottom=1)
        c = d.frame(r, "When", 0, 0, cols[0][1], 52, fill=None, layout="horizontal", padding=[0, 14, 0, 14], gap=8,
                    crossAlign="center", sizingV="fill")
        d.text(c, when, "Body/14 Medium" if cur else "Body/14", "ink", name="Time")
        if cur:
            d.inst("Status badge", c, props={"Tone": "Good", "Label": "Current"}, name="Current")
        c = d.frame(r, "Size", 0, 0, cols[1][1], 52, fill=None, layout="horizontal", padding=[0, 14, 0, 14],
                    crossAlign="center", sizingV="fill")
        d.text(c, size, "Mono/13", "ink", name="Bytes")
        c = d.frame(r, "By", 0, 0, cols[2][1], 52, fill=None, layout="horizontal", padding=[0, 14, 0, 14], gap=8,
                    crossAlign="center", sizingV="fill")
        d.text(c, by, "Body/13", "ink" if "desktop" in by else "muted", name="By name")
        c = d.frame(r, "Actions", 0, 0, cols[3][1], 52, fill=None, layout="horizontal", padding=[0, 14, 0, 14], gap=8,
                    crossAlign="center", mainAlign="max", sizingV="fill", sizingH="fill")
        if not cur:
            d.inst("Button", c, props={"Kind": "Secondary", "Label": "Restore", "State": "Focus" if i == 1 else
                                       "Default"}, name="Restore")
        d.inst("Button", c, props={"Kind": "Secondary", "Label": "Download"}, name="Download")
    dialog_actions(d, dl, [("Primary", "Close")], w)
    return f


# --- 7–10. web pages ------------------------------------------------------------------------------------------

def get_desktop(d, name, x, y):
    f, _ = web_page(d, name, x, y, "Files")
    page_title(d, f, "Get the desktop app", kicker=f"CraftCloud desktop client {VERSION}")
    d.text(f, "The client keeps a CraftCloud folder on your computer in sync with this server. Open and save files "
              "there with the Craft apps (or any app); changes go up and down on their own.", "Body/14", "muted",
           x=24, y=150, w=900, name="Intro")
    grid_y = 200
    cw = (W - 48 + 2) / 3
    oses = [
        ("Linux", "Detected", [("Primary", "AppImage"), ("Secondary", ".deb"), ("Secondary", "Flatpak")],
         "x86_64 and aarch64 · 18 MB",
         "Not signed. AppImage: make it executable (chmod +x CraftCloud.AppImage), then run it. .deb: "
         "sudo apt install ./craftcloud_0.1.0.deb. Flatpak: flatpak install ./CraftCloud.flatpak."),
        ("Windows", None, [("Primary", ".msi installer")], "Windows 10 and 11 · x64 and arm64 · 21 MB",
         "Not signed. SmartScreen may say it protected your PC: choose More info, then Run anyway."),
        ("macOS", None, [("Primary", ".dmg")], "macOS 12 or later · Apple silicon and Intel · 24 MB",
         "Not signed. Gatekeeper blocks a double-click: right-click (or Control-click) CraftCloud in Applications, "
         "choose Open, then Open again. Only the first time."),
    ]
    for i, (os_, badge, btns, meta, note) in enumerate(oses):
        c = d.frame(f, f"Card {os_}", 24 + i * (cw - 1), grid_y, cw, 330, fill="bg", stroke="line", layout="vertical",
                    gap=0, clip=True)
        strip = d.frame(c, "Strip", 0, 0, cw, 40, fill=None, layout="horizontal", padding=[0, 20, 0, 20],
                        crossAlign="center", sizingH="fill")
        side_stroke(d, strip, bottom=1)
        hud(d, strip, VERSION, color="accent_ink", name="Version")
        d.frame(strip, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
        hud(d, strip, "Released 13 Nov 2026", name="Released")
        b = vstack(d, c, "Body", 0, 0, cw, gap=12, pad=[18, 20, 20, 20])
        d.set(b, sizingH="fill", sizingV="fill")
        tr = hstack(d, b, "Title row", gap=10)
        d.text(tr, os_, "Display/Heading 24", "ink_strong", name="OS")
        if badge:
            d.inst("Status badge", tr, props={"Tone": "Accent", "Label": badge}, name="Detected")
        d.text(b, meta, "Small/12.5", "muted", name="Meta")
        br = hstack(d, b, "Downloads", gap=8)
        d.set(br, sizingH="fill")
        for k, (kind, lab) in enumerate(btns):
            bi = d.inst("Button", br, props={"Kind": kind, "Label": lab}, name=lab)
            if k == 0:
                d.set(bi, sizingH="fill")
        nr = hstack(d, b, "Unsigned", gap=8)
        d.inst("Status badge", nr, props={"Tone": "Warn", "Label": "Unsigned"}, name="Unsigned badge")
        d.text(b, note, "Small/12.5", "ink", name="How to open", w=cw - 40)
        d.set(d.sub(b, "How to open"), sizingH="fill")
        if badge:
            rule = d.rect(c, 1, 1, cw - 2, 2, "accent", name="Detected rule")
            d.absolute(rule, 1, 1, cw - 2, 2)
    p = d.frame(f, "Connect a device", 24, grid_y + 330 + 24, W - 48, 10, fill=None, stroke="line",
                layout="horizontal", gap=32, padding=[18, 20, 20, 20], sizingV="hug", crossAlign="min")
    left = vstack(d, p, "Connect left", 0, 0, 640, gap=12)
    hud(d, left, "Connect a device", color="ink", name="Connect title")
    r = hstack(d, left, "URL row", gap=8)
    d.set(r, sizingH="fill")
    i = d.inst("Input", r, props={"Value": URL}, name="Server URL")
    d.set(i, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Copy"}, name="Copy")
    steps = vstack(d, p, "Steps", 0, 0, 600, gap=6)
    d.set(steps, sizingH="fill")
    for n, t in ((1, "Install and start the client."), (2, "Paste the server address above."),
                 (3, "Sign in in the browser window it opens, and confirm the code."),
                 (4, "Pick a local folder and the cloud folders to sync.")):
        rr = hstack(d, steps, f"Step {n}", gap=10)
        hud(d, rr, f"{n:02d}", color="accent_ink", name="n")
        d.text(rr, t, "Body/14", "ink", name="t")
    return f


def admin_subnav(d, f, sel, y=144):
    box = d.frame(f, "Admin sub-nav", 24, y, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                  sizingV="fixed", clip=False)
    for i, lab in enumerate(("Users", "Server")):
        if i:
            d.rect(box, 0, 0, 1, 34, "line", name="sep")
        d.inst("Cell", box, props={"Label": lab, "Show count": False, "State": "On" if lab == sel else "Off"},
               name=f"Sub {lab}")
    return box


def admin_users(d, name, x, y):
    f, hid = web_page(d, name, x, y, "Admin")
    page_title(d, f, "Users", kicker="Admin · 5 people · 61.9 GB of 1 TB")
    admin_subnav(d, f, "Users")
    r = hstack(d, f, "User actions", 0, 88, gap=8)
    s = d.inst("Search field", r, props={"Placeholder": "Search users"}, name="Search users")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Invite by link"}, name="Invite")
    d.inst("Button", r, props={"Kind": "Primary", "Label": "Create user"}, name="Create")
    d.set(r, x=W - 24 - d.R("node.get", {"id": r, "fields": ["width"]})["width"])
    del s
    lw = W - 48
    cols = [("Name", None), ("Username", 170), ("Role", 140), ("Storage used", 260), ("Last seen", 170), ("", 220)]
    lst = vstack(d, f, "Users", 24, 198, lw, gap=0)
    list_header(d, lst, lw, cols, sort="Name")
    users = [("AL", "Ana Lima", "ana", "User", 3.2, 50, "Yesterday", "Good"),
             (USER[0], USER[1], "john", "Admin", 12.4, 100, "Now", "Accent"),
             ("KS", "Kai Sato", "kai", "Invited", 0, 50, "Not yet", "Warn"),
             ("LN", "Lea Novak", "lea", "User", 38.7, 100, "Today 08:10", "Good"),
             ("TB", "Tomas Berg", "tomas", "User", 7.6, 50, "3 Nov 2026", "Good")]
    for i, (ini, nm, un, role, used, quota, seen, tone) in enumerate(users):
        def b0(c, ini=ini, nm=nm):
            av = d.frame(c, "Avatar", 0, 0, 28, 28, fill="sunken", stroke="line_strong", layout="horizontal",
                         mainAlign="center", crossAlign="center")
            d.text(av, ini, "Badge/10", "ink", name="Initials")
            d.text(c, nm, "Body/14", "ink", name="Name")

        def b2(c, role=role, tone=tone):
            d.inst("Status badge", c, props={"Tone": "Accent" if role == "Admin" else ("Warn" if role == "Invited"
                                                                                         else "Muted"),
                                             "Label": role}, name="Role")

        def b3(c, used=used, quota=quota):
            d.set(c, layout="vertical", gap=6, mainAlign="center", crossAlign="min")
            d.text(c, f"{used} GB of {quota} GB" if used else f"— of {quota} GB", "Small/12", "ink", name="Used")
            bar = d.frame(c, "Meter", 0, 0, 200, 4, fill="line")
            if used:
                d.rect(bar, 0, 0, max(2, round(200 * used / quota)), 4, "warn" if used / quota > 0.35 else "accent",
                       name="Fill")

        def b5(c, i=i, role=role):
            d.set(c, mainAlign="max")
            lab = "Resend invite" if role == "Invited" else "Reset password"
            if i == 2:
                d.inst("Button", c, props={"Kind": "Secondary", "Label": lab, "State": "Hover"}, name=lab)
            else:
                d.text(c, lab, "Button/11", "accent_ink", name=lab)
            d.text(c, "…", "Body/14 Medium", "muted", name="More")
        generic_row(d, lst, lw, [(None, b0), (170, lambda c, un=un: d.text(c, un, "Mono/13", "ink", name="Username")),
                                 (140, b2), (260, b3),
                                 (170, lambda c, seen=seen: d.text(c, seen, "Body/13", "muted", name="Seen")),
                                 (220, b5)], h=56, state="Hover" if i == 2 else "Default", name=f"User {nm}")
    d.text(f, "Reset password sets a one-time password and shows it once; the person picks a new one at sign-in. "
              "Quotas and the rarer settings are in OxiCloud's admin pages.", "Small/12.5", "muted", x=24,
           y=198 + 40 + 5 * 56 + 16, name="Users note")
    return f


def web_settings(d, name, x, y):
    f, hid = web_page(d, name, x, y, "Files")
    d.R("instance.setProperties", {"ids": [d.sub(hid, "Nav Files")], "properties": {"State": "Off"}})
    page_title(d, f, "Settings", kicker=f"{USER[1]} · {SERVER}")
    body = d.frame(f, "Settings body", 24, 160, W - 48, 10, fill=None, layout="horizontal", gap=24, sizingV="hug",
                   crossAlign="min", clip=False)
    colw = (W - 48 - 24) / 2
    left = vstack(d, body, "Left", 0, 0, colw, gap=24)
    right = vstack(d, body, "Right", 0, 0, colw, gap=24)
    p = panel(d, left, "Appearance", colw)
    onoff(d, p, "Dark", labels=("System", "Dark", "Light"), name="Theme")
    p = panel(d, left, "Devices", colw)
    d.text(p, "Desktop clients signed in to your account. Revoking signs the device out; its local folder stays.",
           "Small/12.5", "muted", name="Devices note", w=colw - 36)
    tb = vstack(d, p, "Device list", 0, 0, colw - 36, gap=0)
    d.set(tb, sizingH="fill")
    for dev, osn, ver, last in (("Studio", "Linux · Fedora 43", "0.1.0", "Synced 2 min ago"),
                                ("Laptop", "macOS 15", "0.1.0", "Synced 1 h ago"),
                                ("Studio PC", "Windows 11", "0.1.0", "Last seen 9 days ago")):
        r = hstack(d, tb, f"Device {dev}", gap=10, h=54)
        d.set(r, sizingH="fill")
        side_stroke(d, r, top=1)
        col = vstack(d, r, "Device text", 0, 0, 300, gap=2)
        d.set(col, sizingH="fill")
        hr = hstack(d, col, "Device head", gap=8)
        d.text(hr, dev, "Body/14 Medium", "ink", name="Device name")
        if dev == "Studio":
            d.inst("Status badge", hr, props={"Tone": "Good", "Label": "This browser's computer"}, name="Here")
        d.text(col, f"{osn} · client {ver} · {last}", "Small/12", "muted", name="Device meta")
        d.inst("Button", r, props={"Kind": "Secondary", "Label": "Revoke"}, name="Revoke")
    d.text(right, "Server settings (web apps, versions, versions of the software) moved to Admin › Server.",
           "Small/12.5", "muted", name="Moved note", w=colw)
    return f


def admin_server(d, name, x, y):
    f, _ = web_page(d, name, x, y, "Admin")
    page_title(d, f, "Server", kicker=f"Admin · {SERVER}")
    admin_subnav(d, f, "Server")
    body = d.frame(f, "Server body", 24, 198, W - 48, 10, fill=None, layout="horizontal", gap=24, sizingV="hug",
                   crossAlign="min", clip=False)
    colw = (W - 48 - 24) / 2
    left = vstack(d, body, "Left", 0, 0, colw, gap=24)
    right = vstack(d, body, "Right", 0, 0, colw, gap=24)
    p = panel(d, left, "Web apps", colw, admin=True)
    d.text(p, "Host the 15 Craft apps in the browser, so files open without installing anything. Turning this on "
              "adds an APPS entry to the header for everyone.", "Small/12.5", "muted", name="Web apps intro",
           w=colw - 36)
    r = hstack(d, p, "Live toggle", gap=14)
    onoff(d, r, False, name="Web apps")
    d.text(r, f"Off · this server runs craftcloud:{VERSION}-apps", "Body/14 Medium", "ink", name="Toggle state")
    d.text(p, "Changes apply immediately. Stored on the server in /app/storage/.craftcloud/settings.env.",
           "Small/12.5", "muted", name="Toggle note", w=colw - 36)
    vb = vstack(d, p, "Disabled variant", 0, 0, colw - 36, gap=8, pad=[12, 14, 12, 14], fill="sunken")
    d.set(vb, sizingH="fill")
    hud(d, vb, f"On a slim image (craftcloud:{VERSION})", name="Variant label")
    r = hstack(d, vb, "Disabled toggle", gap=14)
    onoff(d, r, False, name="Web apps (disabled)", disabled=True)
    d.inst("Status badge", r, props={"Tone": "Warn", "Label": "Needs the -apps image"}, name="Needs badge")
    d.text(vb, "The -apps image: about 770 MB download, 2.1 GB on disk. CRAFTCLOUD_WEB_APPS only seeds the first "
               "start; after that the stored setting wins.", "Small/12.5", "muted", name="Variant note",
           w=colw - 64)
    p = panel(d, right, "Versions", colw, admin=True)
    kv(d, p, "Versions kept", "5 per file (.craftcloud/versions/)")
    d.text(p, "Before any overwrite, from a desktop client or the web, the old file is kept.", "Small/12.5",
           "muted", name="Versions note", w=colw - 36)
    p = panel(d, right, "About", colw)
    kv(d, p, "CraftCloud", f"{VERSION} (-apps image)", mono=True)
    kv(d, p, "OxiCloud", f"{OXICLOUD} (backend, unmodified)", mono=True)
    kv(d, p, "Public URL", URL, mono=True)
    kv(d, p, "Desktop client", "0.1.0 is the latest", mono=True)
    d.text(p, "CraftCloud is open source (MIT OR Apache-2.0). Fonts: Inter, Archivo, Geist Mono (SIL OFL 1.1). "
              "Look based on ArtCraft Launcher by the ArtCraft team.", "Small/12", "muted", name="Licences",
           w=colw - 36)
    return f


def web_launcher(d, name, x, y):
    f, _ = launcher_grid(d, name, x, y)
    old = d.sub(f, "Header")
    d.set(old, visible=False, name="Header (pre-2026-11-13)")
    hid = d.inst("Header/Web with apps", f, 0, 0, name="Header")
    d.R("node.reorder", {"id": hid, "index": 9999})
    # Label: this view exists only with web apps on.
    tag = hstack(d, f, "Web apps label", 0, 126, gap=8)
    d.inst("Status badge", tag, props={"Tone": "Accent", "Label": "Web apps on"}, name="On badge")
    d.text(tag, "This page and the APPS entry appear only when an admin turns web apps on in Admin › Server "
                "(needs the -apps image).", "Small/12", "muted", name="On note")
    d.set(tag, x=W / 2 - 300)
    return f


# --- 11–16. desktop client -----------------------------------------------------------------------------------

def desk(d, name, x, y, w=DW, h=DH, tab="Activity", status=("Good", "Up to date"), onboarding=False,
         right="Last sync 14:32"):
    f = screen(d, name, x, y, w, h)
    tb = d.inst("Desktop/Title bar", f, 0, 0, name="Title bar")
    if w != DW:
        d.set(tb, width=w)
    if onboarding:
        for k in ("Tabs", "Status", "Account"):
            d.set(d.sub(tb, k), visible=False)
    else:
        for lab in ("Activity", "Folders", "Settings"):
            d.R("instance.setProperties", {"ids": [d.sub(tb, f"Tab {lab}")],
                                           "properties": {"State": "On" if lab == tab else "Off"}})
        d.R("instance.setProperties", {"ids": [d.sub(tb, "Sync badge")],
                                       "properties": {"Tone": status[0], "Label": status[1]}})
    sb = d.inst("Desktop/Status bar", f, 0, h - 34, name="Status bar")
    if w != DW:
        d.set(sb, width=w)
    d.R("instance.setProperties", {"ids": [sb], "properties": {"Right": right if not onboarding else "Not connected"}})
    return f


def desk_steps(d, f, step, x0, w):
    ind = d.frame(f, "Step indicator", x0, 72, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                  sizingV="fixed", clip=False)
    for i, lab in enumerate(("Server", "Sign in", "Folder"), 1):
        if i > 1:
            d.rect(ind, 0, 0, 1, 34, "line", name="sep")
        c = d.inst("Cell", ind, props={"Label": f"{i:02d} {lab}", "Show count": False,
                                       "State": "On" if i == step else "Off"}, name=f"Step {lab}")
        if i < step:
            d.set(d.sub(c, "Label"), fill="good")
    return vstack(d, f, "Body", x0, 130, w, gap=18)


def signin_1(d, name, x, y):
    f = desk(d, name, x, y, onboarding=True)
    w = 560
    b = desk_steps(d, f, 1, (DW - w) / 2, w)
    d.text(b, "Connect to your CraftCloud", "Display/Sheet title 26", "ink_strong", name="Title")
    field(d, b, "Server address", SERVER, state="Focus", w=w,
          hint="The address your admin gave you. It is also on the web app's Get the desktop app page.")
    r = hstack(d, b, "Found", gap=10)
    d.inst("Status badge", r, props={"Tone": "Good", "Label": "Found"}, name="Found badge")
    d.text(r, f"CraftCloud {VERSION} at {URL}", "Small/12.5", "ink", name="Found text")
    act = d.frame(b, "Actions", 0, 0, w, 52, fill=None, layout="horizontal", gap=8, padding=[18, 0, 0, 0],
                  sizingH="fill", sizingV="hug", crossAlign="center")
    side_stroke(d, act, top=1)
    d.frame(act, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", act, props={"Kind": "Primary", "Label": "Continue"}, name="Continue")
    return f


def signin_2(d, name, x, y):
    f = desk(d, name, x, y, onboarding=True)
    w = 560
    b = desk_steps(d, f, 2, (DW - w) / 2, w)
    d.text(b, "Continue in your browser", "Display/Sheet title 26", "ink_strong", name="Title")
    d.text(b, f"We opened {SERVER} in your browser. Sign in there and check that it shows this code:", "Body/14",
           "ink", name="Intro", w=w)
    code = d.frame(b, "Code", 0, 0, w, 72, fill="sunken", stroke="line", layout="horizontal", mainAlign="center",
                   crossAlign="center", sizingH="fill")
    d.text(code, "KQ7-4XM", "Display/Page title 32", "ink_strong", name="Code text")
    wait = vstack(d, b, "Waiting", 0, 0, w, gap=8)
    d.set(wait, sizingH="fill")
    hud(d, wait, "Waiting for the browser…", color="accent_ink", name="Waiting label")
    tr = d.frame(wait, "Track", 0, 0, w, 2, fill="line", sizingH="fill")
    seg = d.rect(tr, 160, 0, 140, 2, "accent", name="Indeterminate segment")
    del seg
    kv(d, b, "This device", "Studio (Linux) · you can rename it later", kw=130)
    act = d.frame(b, "Actions", 0, 0, w, 52, fill=None, layout="horizontal", gap=8, padding=[18, 0, 0, 0],
                  sizingH="fill", sizingV="hug", crossAlign="center")
    side_stroke(d, act, top=1)
    d.inst("Button", act, props={"Kind": "Secondary", "Label": "Back"}, name="Back")
    d.frame(act, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", act, props={"Kind": "Secondary", "Label": "Copy link"}, name="Copy link")
    d.inst("Button", act, props={"Kind": "Primary", "Label": "Open browser again"}, name="Open again")
    return f


def folder_tree(d, b, w, label=True):
    """The cloud folder tree with checkboxes and sizes (onboarding step 3 and the Folders tab)."""
    if label:
        hud(d, b, "Cloud folders", color="ink", name="Tree label")
    tree = vstack(d, b, "Tree", 0, 0, w, gap=0)
    d.set(tree, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [tree], "field": "strokes/0/color", "variable": "line"})
    items = [(0, "Documents", "On", "2.1 GB", True), (1, "Reports", "On", "38 MB", False),
             (1, "Archive", "On", "1.4 GB", False), (0, "Photos", "Mixed", "8.9 GB of 21.3 GB", True),
             (1, "2026", "On", "8.9 GB", False), (1, "2025", "Off", "12.4 GB", False),
             (0, "Projects", "On", "1.2 GB", False), (0, "Video", "Off", "14.2 GB", False),
             (0, "Shared with me", "On", "340 MB", False)]
    for i, (lvl, nm, st, size, open_) in enumerate(items):
        r = hstack(d, tree, f"Node {nm}", gap=10, h=32, padding=[0, 14, 0, 14 + lvl * 24])
        d.set(r, sizingH="fill", fill="sunken" if i == 3 else None)
        if i:
            side_stroke(d, r, top=1)
        d.text(r, "▸" if False else ("–" if open_ else ("+" if lvl == 0 else " ")), "Mono/13", "muted", name="Twisty")
        d.inst("Checkbox", r, props={"State": st}, name="Check")
        d.inst("Icon/Folder", r, name="Folder")
        nt = d.text(r, nm, "Body/13", "ink" if st != "Off" else "muted", name="Folder name")
        d.set(nt, sizingH="fill")
        d.text(r, size, "Mono/12", "muted", name="Size")
    return tree


def desktop_folders(d, name, x, y):
    f = desk(d, name, x, y, tab="Folders")
    w = DW - 48
    b = vstack(d, f, "Body", 24, 72, w, gap=14)
    r = hstack(d, b, "Local folder", gap=10)
    d.set(r, sizingH="fill")
    hud(d, r, "Local folder", color="ink", name="Local label")
    t = d.text(r, "/home/john/CraftCloud", "Mono/13", "ink", name="Local path")
    d.set(t, sizingH="fill")
    d.text(r, "212 GB free on this disk", "Small/12.5", "muted", name="Free space")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Open folder"}, name="Open folder")
    folder_tree(d, b, w)
    act = d.frame(b, "Actions", 0, 0, w, 52, fill=None, layout="horizontal", gap=8, padding=[14, 0, 0, 0],
                  sizingH="fill", sizingV="hug", crossAlign="center")
    side_stroke(d, act, top=1)
    d.text(act, "Syncing 12.9 GB · 3,412 files. Unticking a folder removes its local copy; the cloud keeps it.",
           "Small/12.5", "muted", name="Total")
    d.frame(act, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", act, props={"Kind": "Secondary", "Label": "Revert"}, name="Revert")
    d.inst("Button", act, props={"Kind": "Primary", "Label": "Apply"}, name="Apply")
    return f


def signin_3(d, name, x, y):
    f = desk(d, name, x, y, h=720, onboarding=True)  # 960×720: the folder tree needs the room
    w = 680
    b = desk_steps(d, f, 3, (DW - w) / 2, w)
    d.set(b, gap=14)
    d.text(b, "Choose what to sync", "Display/Sheet title 26", "ink_strong", name="Title")
    field(d, b, "Local folder", "~/CraftCloud", w=w, button="Choose…",
          hint="/home/john/CraftCloud · 212 GB free on this disk")
    folder_tree(d, b, w)
    act = d.frame(b, "Actions", 0, 0, w, 52, fill=None, layout="horizontal", gap=8, padding=[14, 0, 0, 0],
                  sizingH="fill", sizingV="hug", crossAlign="center")
    side_stroke(d, act, top=1)
    d.inst("Button", act, props={"Kind": "Secondary", "Label": "Back"}, name="Back")
    d.text(act, "Downloads 12.9 GB · 3,412 files", "Small/12.5", "muted", name="Total")
    d.frame(act, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", act, props={"Kind": "Primary", "Label": "Start sync"}, name="Start sync")
    return f


ACTIVITY = [("Uploaded", "Good", "Q3 report.docx", "Documents/Reports", "Studio → cloud", "14:32"),
            ("Version saved", "Muted", "Q3 report.docx", "previous version kept (5 of 5)", "cloud", "14:32"),
            ("Downloaded", "Accent", "Poster.psd", "Design", "from Laptop", "11:20"),
            ("Conflict", "Warn", "Budget 2027.xlsx", "kept both: Budget 2027 (conflict 2026-11-13 10.41 Studio).xlsx",
             "", "10:41"),
            ("Downloaded", "Accent", "Logo.svg", "Design", "web upload", "10:02"),
            ("Uploaded", "Good", "Contract.pdf", "Documents", "Studio → cloud", "08:30"),
            ("Deleted", "Muted", "Old budget.xlsx", "Finance · moved to the cloud trash", "", "09:12")]


def desktop_main(d, name, x, y):
    f = desk(d, name, x, y, status=("Accent", "Syncing 3 files"), right="Syncing · 41 MB of 58 MB")
    # Status panel.
    st = d.frame(f, "Status", 24, 72, DW - 48, 10, fill=None, stroke="line", layout="vertical", gap=12,
                 padding=[18, 20, 18, 20], sizingV="hug", clip=False)
    r = hstack(d, st, "Status head", gap=12)
    d.set(r, sizingH="fill")
    d.text(r, "Syncing 3 files", "Display/Heading 24", "ink_strong", name="Status title")
    d.frame(r, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Pause sync"}, name="Pause")
    pr = vstack(d, st, "Progress", 0, 0, DW - 88, gap=6)
    d.set(pr, sizingH="fill")
    d.text(pr, "Poster.psd · 41 MB of 58 MB · about 20 s left", "Small/12.5", "ink", name="Progress text")
    tr = d.frame(pr, "Track", 0, 0, DW - 88, 4, fill="line", sizingH="fill")
    d.rect(tr, 0, 0, round((DW - 88) * 0.7), 4, "accent", name="Done")
    info = hstack(d, st, "Info row", gap=0)
    d.set(info, sizingH="fill")
    for k, v in (("Folder", "~/CraftCloud"), ("Server", f"{SERVER} · {USER[1]}"), ("Synced", "3,412 files · 12.9 GB")):
        c = vstack(d, info, f"Info {k}", 0, 0, 280, gap=4)
        d.set(c, sizingH="fill")
        hud(d, c, k, name="k")
        d.text(c, v, "Mono/12" if k == "Folder" else "Body/13", "ink", name="v")
    d.inst("Button", info, props={"Kind": "Primary", "Label": "Open folder"}, name="Open folder")
    # Activity list.
    hud(d, f, "Activity · today", 24, 248, color="ink", name="Activity label")
    lst = vstack(d, f, "Activity list", 24, 270, DW - 48, gap=0)
    d.set(lst, strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [lst], "field": "strokes/0/color", "variable": "line"})
    for i, (kind, tone, fn, where, who, when) in enumerate(ACTIVITY):
        r = hstack(d, lst, f"Act {i}", gap=12, h=44, padding=[0, 16, 0, 16])
        d.set(r, sizingH="fill")
        if i:
            side_stroke(d, r, top=1)
        bd = d.inst("Status badge", r, props={"Tone": tone, "Label": kind}, name="Kind")
        d.set(bd, sizingH="fixed", width=110)
        col = hstack(d, r, "What", gap=8)
        d.set(col, sizingH="fill")
        d.text(col, fn, "Body/13 Medium", "ink", name="File")
        t = d.text(col, where + (f" · {who}" if who else ""), "Small/12", "muted", name="Where")
        del t
        d.text(r, when, "Mono/12", "muted", name="When")
        if kind == "Conflict":
            d.text(r, "Review", "Button/11", "accent_ink", name="Review")
    return f


def desktop_conflict(d, name, x, y):
    f = desk(d, name, x, y, status=("Warn", "1 conflict"))
    hud(d, f, "Activity · today", 24, 72, color="ink", name="Activity label")
    scrim(d, f, DW, DH)
    w = 700
    dl, body = dialog(d, f, (DW - w) / 2, 64, w, "Q3 report.docx changed in two places", hud_label="Conflict")
    d.text(body, "It was saved on this computer and on Laptop before either change reached the other. Nothing is "
                 "lost: pick what to keep.", "Body/14", "muted", name="Intro", w=w - 48)
    two = d.frame(body, "Both versions", 0, 0, w - 48, 10, fill=None, layout="horizontal", gap=0, sizingH="fill",
                  sizingV="hug", clip=False)
    for lab, dev, when, size in (("Mine", "Studio (this computer)", "Today 14:32", "252 KB"),
                                 ("Cloud", "Laptop", "Today 14:29", "249 KB")):
        c = vstack(d, two, f"Version {lab}", 0, 0, (w - 48) / 2, gap=6, pad=[14, 16, 14, 16])
        d.set(c, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
        d.R("variable.bind", {"ids": [c], "field": "strokes/0/color", "variable": "line"})
        hud(d, c, lab, color="accent_ink" if lab == "Mine" else "ink", name="Which")
        d.text(c, dev, "Body/14 Medium", "ink", name="Device")
        d.text(c, f"Saved {when} · {size}", "Small/12.5", "muted", name="Meta")
    hud(d, body, "Keep", color="ink", name="Keep label")
    onoff(d, body, "Keep both", labels=("Keep both", "Keep mine", "Keep cloud"), name="Choice")
    nb = vstack(d, body, "Choice explain", 0, 0, w - 48, gap=6, pad=[12, 14, 12, 14], fill="sunken")
    d.set(nb, sizingH="fill")
    d.text(nb, "The cloud version stays Q3 report.docx. Yours is saved next to it as:", "Small/12.5", "ink",
           name="Explain")
    d.text(nb, "Q3 report (conflict 2026-11-13 14.32 Studio).docx", "Mono/13", "ink_strong", name="Copy name")
    d.text(body, "Keep mine or Keep cloud replaces the other version; the replaced file goes to "
                 ".craftcloud/versions/ first.", "Small/12.5", "muted", name="Versions note", w=w - 48)
    row = d.frame(dl, "Dialog actions", 0, 0, w, 66, fill=None, layout="horizontal", gap=8, padding=[16, 24, 16, 24],
                  crossAlign="center", sizingH="fill", sizingV="hug")
    side_stroke(d, row, top=1)
    d.inst("Button", row, props={"Kind": "Secondary", "Label": "Decide later"}, name="Later")
    d.frame(row, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Button", row, props={"Kind": "Secondary", "Label": "Open both"}, name="Open both")
    d.inst("Button", row, props={"Kind": "Primary", "Label": "Keep both", "State": "Focus"}, name="Apply")
    return f


def desktop_settings(d, name, x, y):
    h = 780
    f = desk(d, name, x, y, h=h, tab="Settings")
    body = d.frame(f, "Body", 24, 72, DW - 48, 10, fill=None, layout="horizontal", gap=20, sizingV="hug",
                   crossAlign="min", clip=False)
    colw = (DW - 48 - 20) / 2
    left = vstack(d, body, "Left", 0, 0, colw, gap=16)
    right = vstack(d, body, "Right", 0, 0, colw, gap=16)
    p = panel(d, left, "General", colw, pad=(14, 16, 16, 16), gap=10)
    for lab, w_, lbl in (("Start at login", True, ("Off", "On")),):
        r = hstack(d, p, lab, gap=12)
        t = d.text(r, lab, "Body/13", "ink", name="Label")
        d.set(t, sizingH="fixed", width=130)
        onoff(d, r, w_, labels=lbl, name=lab)
    r = hstack(d, p, "Appearance", gap=12)
    t = d.text(r, "Appearance", "Body/13", "ink", name="Label")
    d.set(t, sizingH="fixed", width=130)
    onoff(d, r, "System", labels=("System", "Dark", "Light"), name="Theme")
    r = hstack(d, p, "Language", gap=12)
    d.set(r, sizingH="fill")
    t = d.text(r, "Language", "Body/13", "ink", name="Label")
    d.set(t, sizingH="fixed", width=130)
    s = d.inst("Select", r, props={"Value": "English (system)"}, name="Language picker")
    d.set(s, sizingH="fill")
    p = panel(d, left, "Sync", colw, pad=(14, 16, 16, 16), gap=10)
    r = hstack(d, p, "Syncing", gap=12)
    t = d.text(r, "Syncing", "Body/13", "ink", name="Label")
    d.set(t, sizingH="fixed", width=130)
    onoff(d, r, True, labels=("Paused", "Active"), name="Syncing")
    r = hstack(d, p, "Folder", gap=8)
    d.set(r, sizingH="fill")
    t = d.text(r, "~/CraftCloud", "Mono/12", "ink", name="Folder path")
    d.set(t, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Move…"}, name="Move")
    r = hstack(d, p, "Selective", gap=8)
    d.set(r, sizingH="fill")
    t = d.text(r, "Syncing 6 of 9 cloud folders · 12.9 GB", "Body/13", "ink", name="Selective text")
    d.set(t, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Choose folders…"}, name="Choose")
    for lab, val in (("Upload limit", "No limit"), ("Download limit", "2 MB/s")):
        r = hstack(d, p, lab, gap=12)
        d.set(r, sizingH="fill")
        t = d.text(r, lab, "Body/13", "ink", name="Label")
        d.set(t, sizingH="fixed", width=130)
        s = d.inst("Select", r, props={"Value": val}, name=f"{lab} picker")
        d.set(s, sizingH="fill")
    p = panel(d, right, "Versions", colw, pad=(14, 16, 16, 16), gap=8)
    d.text(p, "Before the client overwrites a file, the old one goes to .craftcloud/versions/ on the server. The last "
              "5 are kept; restore them from the web app.", "Small/12.5", "muted", name="Versions text", w=colw - 32)
    d.inst("Button", p, props={"Kind": "Secondary", "Label": "Open versions in browser ↗"}, name="Versions link")
    p = panel(d, right, "Updates", colw, pad=(14, 16, 16, 16), gap=8)
    r = hstack(d, p, "Update state", gap=8)
    d.inst("Status badge", r, props={"Tone": "Good", "Label": "Up to date"}, name="Update badge")
    d.text(r, f"{VERSION} · checked 10 min ago", "Small/12.5", "ink", name="Update text")
    r = hstack(d, p, "Update actions", gap=8)
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Check now"}, name="Check now")
    onoff(d, r, False, labels=("Auto off", "Auto on"), name="Auto update")
    d.text(p, "Updates come from the server's desktop-app page (craft-update).", "Small/12", "muted", name="Upd note")
    p = panel(d, right, "Account", colw, pad=(14, 16, 16, 16), gap=8)
    kv(d, p, "Signed in", f"{USER[1]} · {SERVER}", kw=110)
    kv(d, p, "This device", "Studio", kw=110)
    d.inst("Button", p, props={"Kind": "Secondary", "Label": "Sign out"}, name="Sign out")
    p = panel(d, right, "About", colw, pad=(14, 16, 16, 16), gap=6)
    kv(d, p, "CraftCloud", f"{VERSION} · Linux x86_64", kw=110, mono=True)
    d.text(p, "MIT OR Apache-2.0 · Look based on ArtCraft Launcher by the ArtCraft team.", "Small/12", "muted",
           name="Licence")
    return f


def tray_icon(d, parent, state, x, y, size=22):
    box = d.frame(parent, f"Tray {state}", x, y, size + 6, size + 6, fill=None, clip=False)
    c = d.inst("Icon/Cloud", box, 3, 3, name="Cloud")
    col = {"Synced": "good", "Syncing": "accent", "Paused": "warn", "Error": "bad"}[state]
    dot = d.node("ellipse", box, "Badge", size - 6, size - 6, 12, 12, fill=col, stroke="bg", strokeWeight=1.5,
                 strokeAlign="outside")
    if state == "Synced":
        ck = d.inst("Icon/Check", box, size - 6, size - 6, name="Tick")
        d.set(ck, width=12, height=12)
        d.set(d.sub(ck, "glyph"), stroke="#0b0b0c", strokeWeight=1.5)
    elif state == "Paused":
        d.rect(box, size - 3, size - 3, 2, 6, "#0b0b0c", name="bar1")
        d.rect(box, size + 1, size - 3, 2, 6, "#0b0b0c", name="bar2")
    elif state == "Error":
        d.rect(box, size - 0.75, size - 3.5, 1.5, 4.5, "#ffffff", name="excl")
        d.rect(box, size - 0.75, size + 2, 1.5, 1.5, "#ffffff", name="dot")
    else:
        d.node("vector", box, "Arrows", size - 3, size - 3, path="M0 3 L3 0 L6 3 M3 0 L3 6", closed=False,
               fill=None, stroke="#ffffff", strokeWeight=1.25)
    del c, dot
    return box


def desktop_tray(d, name, x, y):
    w, h = 560, 760
    f = screen(d, name, x, y, w, h)
    d.set(f, fill="sunken")
    bar = d.frame(f, "System tray", 0, 0, w, 32, fill="bg", layout="horizontal", padding=[0, 12, 0, 12], gap=10,
                  crossAlign="center", mainAlign="max")
    side_stroke(d, bar, bottom=1)
    d.text(bar, "Fri 13 Nov  14:33", "Small/12", "ink", name="Clock")
    sel = d.frame(bar, "Tray icon (open)", 0, 0, 30, 30, fill="invert_bg", layout="horizontal", mainAlign="center",
                  crossAlign="center")
    ic = d.inst("Icon/Cloud", sel, name="Cloud")
    d.set(d.sub(ic, "glyph"), fill="invert_fg")
    mw = 280
    m = d.frame(f, "Tray menu", w - 12 - mw, 38, mw, 10, fill="bg", stroke="line", layout="vertical",
                padding=[0, 0, 4, 0], sizingV="hug", clip=False)
    d.effect_style(m, "Shadow/Dialog")
    head = vstack(d, m, "Status line", 0, 0, mw, gap=4, pad=[12, 14, 12, 14])
    d.set(head, sizingH="fill")
    side_stroke(d, head, bottom=1)
    r = hstack(d, head, "Status row", gap=8)
    d.inst("Status badge", r, props={"Tone": "Good", "Label": "Up to date"}, name="Status badge")
    d.text(r, "14:32", "Mono/12", "muted", name="Time")
    d.text(head, f"{USER[1]} · {SERVER}", "Small/12", "muted", name="Who")

    def item(lab, trail=None, tone=None, hover=False):
        it = d.inst("Menu item", m, props={"Label": lab, "Icon": False, "Show trail": bool(trail),
                                           "State": "Hover" if hover else "Default", **({"Trail": trail} if trail
                                                                                       else {})}, name=f"Item {lab}")
        d.set(it, sizingH="fill")
        return it

    def sep():
        s = d.frame(m, "Separator", 0, 0, mw, 9, fill=None, layout="vertical", padding=[4, 0, 4, 0], sizingH="fill")
        rr = d.rect(s, 0, 0, mw, 1, "line", name="rule")
        d.set(rr, sizingH="fill")

    d.set(m, padding=[0, 0, 4, 0])
    item("Open CraftCloud folder", hover=True)
    item("Open in browser", "↗")
    item("Pause syncing")
    sep()
    hh = d.frame(m, "Recent head", 0, 0, mw, 28, fill=None, layout="horizontal", padding=[0, 12, 0, 12],
                 crossAlign="center", sizingH="fill")
    hud(d, hh, "Recent files", name="Recent label")
    for fn, t in (("Q3 report.docx", "14:32"), ("Poster.psd", "11:20"), ("Logo.svg", "10:02"),
                  ("Budget 2027.xlsx", "Yesterday"), ("Floorplan.dwg", "Wed")):
        item(fn, t)
    sep()
    item("Settings…")
    item("Quit CraftCloud", "Ctrl+Q")
    # Icon states.
    st = d.frame(f, "Tray icon states", 24, h - 150, w - 48, 120, fill="bg", stroke="line", layout="horizontal",
                 gap=0, clip=False)
    for i, s in enumerate(("Synced", "Syncing", "Paused", "Error")):
        c = vstack(d, st, f"State {s}", 0, 0, (w - 48) / 4, gap=10, pad=[18, 0, 18, 0])
        d.set(c, sizingH="fill", sizingV="fill", crossAlign="center")
        if i:
            side_stroke(d, c, left=1)
        tray_icon(d, c, s, 0, 0, size=22)
        hud(d, c, s, color="ink", name="State label")
        d.text(c, {"Synced": "up to date", "Syncing": "3 files", "Paused": "by you", "Error": "needs you"}[s],
               "Small/12", "muted", name="State note")
    hud(d, f, "Tray icon states (22 px, monochrome cloud + status dot)", 24, h - 176, name="States label")
    return f


def desktop_states(d, name, x, y):
    h = 760
    f = screen(d, name, x, y, DW, h)
    d.text(f, "Desktop client states", "Display/Heading 24", "ink_strong", x=24, y=24, name="Title")
    cw, ch = (DW - 48 - 16) / 2, 190
    panels = [
        ("Bad", "Offline", "No network", "Changes are kept and go up when you're back online. 4 files waiting.",
         [("Secondary", "Retry now")]),
        ("Warn", "Signed out", "Sign in again", f"Your session on {SERVER} ended (password changed or the device was "
                                               "revoked). Sync is stopped; nothing was deleted.",
         [("Primary", "Sign in in browser")]),
        ("Bad", "Disk full", "Not enough space on this computer", "Poster.psd (58 MB) couldn't download: 31 MB free "
                                                                   "on /home. Free space or sync fewer folders.",
         [("Secondary", "Choose folders…"), ("Secondary", "Retry")]),
        ("Bad", "Server unreachable", f"{SERVER} doesn't answer", "Last answer 14:02. The client retries every "
                                                                 "minute; local changes are safe.",
         [("Secondary", "Check status page ↗"), ("Secondary", "Retry now")]),
        ("Warn", "File in use", "Q3 report.docx is open in WordCraft", "A newer version is in the cloud. It downloads "
                                                                       "when the file is closed, or saves next to it as "
                                                                       "a conflict copy.",
         [("Secondary", "Show file")]),
        ("Muted", "Paused", "Syncing is paused", "Paused by you at 13:10. 12 changes are waiting.",
         [("Primary", "Resume")]),
    ]
    for i, (tone, badge, title, text, btns) in enumerate(panels):
        r_, c_ = divmod(i, 2)
        p = d.frame(f, f"State {badge}", 24 + c_ * (cw + 16), 76 + r_ * (ch + 16), cw, ch, fill="bg", stroke="line",
                    layout="vertical", gap=10, padding=[16, 18, 16, 18], clip=True)
        rule = d.rect(p, 0, 0, 2, ch, {"Bad": "bad", "Warn": "warn", "Muted": "line_strong", "Good": "good"}[tone],
                      name="Tone rule")
        d.absolute(rule, 0, 0, 2, ch)
        d.inst("Status badge", p, props={"Tone": tone, "Label": badge}, name="Badge")
        d.text(p, title, "Body/14 Medium", "ink_strong", name="Panel title")
        d.text(p, text, "Small/12.5", "muted", name="Panel text", w=cw - 36)
        d.frame(p, "Spacer", 0, 0, 10, 10, fill=None, sizingV="fill")
        r = hstack(d, p, "Buttons", gap=8)
        for kind, lab in btns:
            d.inst("Button", r, props={"Kind": kind, "Label": lab}, name=lab)
    yb = 76 + 3 * (ch + 16) + 8
    hud(d, f, "Title bar status badge", 24, yb, color="ink", name="Badges label")
    r = hstack(d, f, "Badges", 24, yb + 24, gap=10)
    for tone, lab in (("Good", "Up to date"), ("Accent", "Syncing 3 files"), ("Muted", "Paused"), ("Bad", "Offline"),
                      ("Warn", "1 conflict"), ("Bad", "Error")):
        d.inst("Status badge", r, props={"Tone": tone, "Label": lab}, name=lab)
    return f


# --- build ---------------------------------------------------------------------------------------------------

def build_new(d):
    frames = []

    def add(n, fid):
        frames.append((n, fid))

    d.page("A Setup")
    for i, fn in enumerate((setup_1, setup_2, setup_3, setup_4, setup_5)):
        n = ("setup-1-welcome", "setup-2-admin", "setup-3-server", "setup-4-webapps", "setup-5-done")[i]
        add(n, fn(d, n, i * (W + GAP), 0))
    add("web-login", web_login(d, "web-login", 0, H + GAP))

    pf = d.page("B Web files")
    fd = web_files(d, "web-files-dark", 0, 0)
    add("web-files-dark", fd)
    add("web-files-light", web_files(d, "web-files-light", W + GAP, 0, light=True))
    add("web-search-results", web_search(d, "web-search-results", 2 * (W + GAP), 0))
    add("web-recent", web_recent(d, "web-recent", 0, H + GAP))
    add("web-trash", web_trash(d, "web-trash", W + GAP, H + GAP))
    add("web-share-dialog", share_dialog(d, "web-share-dialog", 0, 2 * (H + GAP), fd))
    add("web-versions-dialog", versions_dialog(d, "web-versions-dialog", W + GAP, 2 * (H + GAP), fd))
    del pf

    d.page("C Web pages")
    add("web-get-desktop-app", get_desktop(d, "web-get-desktop-app", 0, 0))
    add("web-admin-users", admin_users(d, "web-admin-users", W + GAP, 0))
    add("web-settings", web_settings(d, "web-settings", 2 * (W + GAP), 0))
    add("web-admin-server", admin_server(d, "web-admin-server", W + GAP, H + GAP))
    add("web-launcher-grid-dark", web_launcher(d, "web-launcher-grid-dark", 0, H + GAP))

    d.page("D Desktop client")
    sx = DW + 120
    add("desktop-signin-1-server", signin_1(d, "desktop-signin-1-server", 0, 0))
    add("desktop-signin-2-browser", signin_2(d, "desktop-signin-2-browser", sx, 0))
    add("desktop-signin-3-folder", signin_3(d, "desktop-signin-3-folder", 2 * sx, 0))
    add("desktop-main", desktop_main(d, "desktop-main", 0, DH + 120))
    add("desktop-conflict", desktop_conflict(d, "desktop-conflict", sx, DH + 120))
    add("desktop-settings", desktop_settings(d, "desktop-settings", 2 * sx, DH + 120))
    add("desktop-tray-menu", desktop_tray(d, "desktop-tray-menu", 0, 2 * (DH + 120) + 240))
    add("desktop-states", desktop_states(d, "desktop-states", sx, 2 * (DH + 120) + 240))
    add("desktop-folders", desktop_folders(d, "desktop-folders", 2 * sx, 2 * (DH + 120) + 240))
    return frames
