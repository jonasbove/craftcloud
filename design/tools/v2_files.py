"""CraftCloud v2 · Files: the web Files page (crates/ui-egui/src/web/files.rs and files/), every state of it,
on its own page "v2 · Files".

The 0.1.0 states follow what the app draws today (sidebar 232 at ≥ 900 px, toolbar, meta line, the table with
36 px header and 44 px rows, the sheets, the drop overlay, the uploads panel, the phone layout under 600 px).
What the app doesn't do yet (the details panel, the version history, "Open in <app>", "Share…" and
"Versions…" in the menu) carries a "New in 0.2.0" tag.
"""

from lib import APP, KIND, VERSION
from components import ICON, circle_path
from screens import hud, vstack, hstack, side_stroke, screen, menu
import v2
from v2 import W, H, GAP, HOST, OWNER, ICONS, ICON_BUTTON_KINDS

SIDE = 232          # files.rs: sidebar width when the area is ≥ 900 wide
PAD = 24
X0 = SIDE + PAD     # left edge of the main column
MW = W - X0 - PAD   # main column width (1160)
TOOL_Y = 48 + 24    # toolbar row
META_Y = TOOL_Y + 34 + 14
LIST_Y = META_Y + 28 + 8
ROW_H = 44          # draw.rs ROW_H
HEAD_H = 36         # draw.rs HEADER_H
CHECK_W, MORE_W = 36, 40
FOOT_LEFT = f"CraftCloud {VERSION} · {HOST}"
SEL_WASH = {False: "#5e7a4829", True: "#5e7a481f"}   # draw.rs row_background: accent at 16 % / 12 %
DROP_WASH = "#5e7a4824"                               # files.rs drop_overlay: accent at 14 %
PW, PH = 390, 844   # phone (shots.rs)

STAR = "M7 1.2 L8.7 5 L12.8 5.3 L9.7 8 L10.6 12.1 L7 10 L3.4 12.1 L4.3 8 L1.2 5.3 L5.3 5 Z"
GLYPHS = {
    "up": "M7 12.5 L7 1.8 M2.8 6 L7 1.8 L11.2 6",
    "plus": ICONS["plus"],
    "refresh": ICONS["refresh"],
    "grid": "M2 2 L6 2 L6 6 L2 6 Z M8 2 L12 2 L12 6 L8 6 Z M2 8 L6 8 L6 12 L2 12 Z M8 8 L12 8 L12 12 L8 12 Z",
    "folder": ICONS["folder"],
    "clock": circle_path(7, 7, 5.6, 24) + " M7 3.8 L7 7 L9.4 8.6",
    "star": STAR,
    "trash": "M1.8 3.5 L12.2 3.5 M5 3.5 L5 1.8 L9 1.8 L9 3.5 M3.2 3.5 L3.8 12.5 L10.2 12.5 L10.8 3.5",
    "x": ICONS["x"],
    "check": ICONS["check"],
    "arrow": ICONS["arrow-right"],
    "download": ICONS["download"],
    "logout": ICONS["logout"],
    "link": "M6 8 L8 6 M5.2 5.4 L3.6 7 C2.5 8.1 2.5 9.9 3.6 11 C4.7 12.1 6.5 12.1 7.6 11 L9.2 9.4 "
            "M4.8 4.6 L6.4 3 C7.5 1.9 9.3 1.9 10.4 3 C11.5 4.1 11.5 5.9 10.4 7 L8.8 8.6",
    "share": "M7 9 L7 1.8 M4 4.6 L7 1.8 L10 4.6 M3.5 7 L1.8 7 L1.8 12.5 L12.2 12.5 L12.2 7 L10.5 7",
    "history": circle_path(7.5, 7, 5.2, 24) + " M7.5 4 L7.5 7 L9.6 8.3",
}

# --- demo data ------------------------------------------------------------------------------------------
# name, app ("folder", an APP id or None), type by format, modified, size, starred, shared, location
FILES = [
    ("Drafts", "folder", "Folder", "Today 09:10", "—", False, False, "My files/Documents/Reports"),
    ("Figures", "folder", "Folder", "Yesterday 18:20", "—", True, False, "My files/Documents/Reports"),
    ("Budget 2027.xlsx", "gridcraft", KIND["gridcraft"], "Yesterday 16:05", "86 KB", True, True,
     "My files/Finance"),
    ("Contract.pdf", "pdfcraft", "PDF document", "Today 08:30", "1.2 MB", False, True, "My files/Documents"),
    ("Floorplan.dwg", "cadcraft", "DWG file", "11 Nov 2026", "3.4 MB", False, False, "My files/Projects/Studio"),
    ("Logo.svg", "vectorcraft", "SVG image", "Today 10:02", "14 KB", True, True, "My files/Design"),
    ("Notes.md", "wordcraft", "Markdown", "Today 07:55", "3.0 KB", False, False, "My files/Documents"),
    ("Pitch.pptx", "deckcraft", KIND["deckcraft"], "10 Nov 2026", "5.8 MB", False, True, "My files/Documents"),
    ("Poster.psd", "photocraft", KIND["photocraft"], "Today 11:20", "42.1 MB", False, False, "My files/Design"),
    ("Q3 report.docx", "wordcraft", KIND["wordcraft"], "Today 11:42", "248 KB", False, True,
     "My files/Documents/Reports"),
    ("Theme.wav", "soundcraft", "WAV audio", "9 Nov 2026", "31.0 MB", False, False, "My files/Audio"),
    ("archive.zip", None, "Archive", "2 Nov 2026", "96.3 MB", False, False, "My files/Documents/Reports"),
]
BY_NAME = {f[0]: f for f in FILES}
CRUMBS = ("My files", "Documents", "Reports")
META = "2 folders · 10 files · 180 MB"
VERSIONS = [  # when, size: the current file, then the 5 kept (.craftcloud/versions/<path>/<time>)
    ("Today 11:42", "248 KB"), ("Today 10:15", "244 KB"), ("Yesterday 17:30", "231 KB"),
    ("Yesterday 09:12", "229 KB"), ("11 Nov 2026 15:48", "210 KB"), ("10 Nov 2026 11:03", "188 KB")]


# --- small parts ----------------------------------------------------------------------------------------

def glyph(d, parent, key, color="ink", name=None, size=14):
    box = d.frame(parent, name or f"Glyph {key}", 0, 0, size, size, fill=None, clip=False)
    solid = key in ("grid",)
    d.node("vector", box, "glyph", (size - 14) / 2, (size - 14) / 2, path=GLYPHS[key], closed=solid,
           fill=color if solid else None, stroke=None if solid else color, strokeWeight=1.5)
    return box


def star(d, parent, color="warn", size=13, outline=False):
    box = d.frame(parent, "Star", 0, 0, size, size, fill=None, clip=False)
    v = d.node("vector", box, "glyph", 0, 0, path=STAR, closed=True, fill=None if outline else color,
               stroke=color if outline else None, strokeWeight=1.3)
    if size != 14:
        d.R("transform.scale", {"ids": [v], "factor": size / 14})
    return box


def new_tag(d, parent, x=None, y=None, text="New in 0.2.0", name="New in 0.2.0"):
    """The design's marker for what 0.1.0 doesn't do: 18 tall, accent wash, hairline, Badge 10 accent ink."""
    t = d.frame(parent, name, x or 0, y or 0, 10, 18, fill="bg", stroke="accent", layout="horizontal",
                padding=[0, 6, 0, 6], crossAlign="center", sizingH="hug", sizingV="fixed", clip=False)
    d.text(t, text, "Badge/10", "accent_ink", name="Tag label")
    return t


def ib(d, parent, label, key, kind="Primary", name=None, disabled=False):
    """widgets.rs icon_label_button: 34 tall, padding 0 16 0 14, 14 px glyph, gap 8, Button 11."""
    fill, stroke, ink = ICON_BUTTON_KINDS[kind]
    b = d.frame(parent, name or label, 0, 0, 10, 34, fill=fill, stroke=stroke, layout="horizontal",
                padding=[0, 16, 0, 14], gap=8, crossAlign="center", sizingH="hug", sizingV="fixed", clip=False)
    glyph(d, b, key, ink)
    d.text(b, label, "Button/11", ink, name="Label")
    if disabled:
        d.set(b, opacity=0.5)
    return b


def square_btn(d, parent, key, on=False, name=None):
    """draw.rs icon_button: 34 × 34, 1 px line_strong, glyph in ink (inverted when on)."""
    s = d.frame(parent, name or f"Button {key}", 0, 0, 34, 34, fill="invert_bg" if on else "bg",
                stroke=None if on else "line_strong", layout="horizontal", mainAlign="center", crossAlign="center")
    glyph(d, s, key, "invert_fg" if on else "ink")
    return s


def btn(d, parent, label, kind="Secondary", name=None, state="Default"):
    return v2.btn(d, parent, label, kind, name=name, state=state)


def search(d, parent, text, w=240, typed=False):
    s = d.inst("Search field", parent, props={"Placeholder": text}, name="Search")
    d.set(s, sizingH="fixed", width=w)
    if typed:
        d.set(d.sub(s, "Placeholder"), fill="ink_strong")
        d.set(s, strokes=[{"type": "solid", "color": "#a9c48e"}], strokeWeight=2)
        d.R("variable.bind", {"ids": [s], "field": "strokes/0/color", "variable": "focus"})
    return s


def crumbs(d, parent, names, own_case_from=1):
    """draw.rs crumbs: hairline cells side by side, the last inverted; "My files" and view names in HUD caps,
    a folder's own name in its own case (Body 13)."""
    r = hstack(d, parent, "Crumbs", gap=0, h=34)
    for i, n in enumerate(names):
        last = i == len(names) - 1
        c = d.frame(r, f"Crumb {n}", 0, 0, 10, 34, fill="invert_bg" if last else "bg", stroke="line",
                    layout="horizontal", padding=[0, 14, 0, 14], crossAlign="center", sizingH="hug",
                    sizingV="fixed")
        ink = "invert_fg" if last else "muted"
        if i >= own_case_from:
            d.text(c, n, "Body/13", ink, name="Crumb label")
        else:
            hud(d, c, n, color=ink, name="Crumb label")
    return r


def check(d, parent, on=False, strong=False):
    if on:
        b = d.frame(parent, "Check", 0, 0, 16, 16, fill="accent", layout="horizontal", mainAlign="center",
                    crossAlign="center")
        g = glyph(d, b, "check", "#ffffff", size=14)
        d.R("transform.scale", {"ids": [g], "factor": 0.8})
        return b
    return d.frame(parent, "Check", 0, 0, 16, 16, fill=None, stroke="line_strong" if strong else "line")


def file_icon(d, parent, app):
    """draw.rs file_glyph: a grey folder, or a page in the colour of the app that opens it."""
    if app == "folder":
        fo = d.inst("Icon/Folder", parent, name="Folder icon")
        d.set(d.sub(fo, "glyph"), fill="muted")
        return fo
    fi = d.inst("Icon/File", parent, name="File icon")
    d.set(d.sub(fi, "page"), fill=f"app:{app}/color" if app else "#8c8c8c")
    return fi


def app_chip(d, parent, app, size=16):
    """draw.rs app_chip: a rounded square in the app colour with its initial, then its name in its ink."""
    r = hstack(d, parent, "App chip", gap=8)
    sq = d.frame(r, "App square", 0, 0, size, size, fill=f"app:{app}/color", layout="horizontal",
                 mainAlign="center", crossAlign="center", cornerRadius=round(size * 0.22))
    d.text(sq, APP[app]["name"][0], "Badge/10", "#ffffff", name="Initial")
    d.text(r, APP[app]["name"], "Body/13", f"app:{app}/ink", name="App name")
    return r


def more_dots(d, parent, color="muted"):
    b = d.frame(parent, "More dots", 0, 0, 14, 4, fill=None, clip=False)
    for i in range(3):
        d.node("ellipse", b, f"dot {i}", i * 5, 0.5, 3, 3, fill=color)
    return b


# --- page chrome ----------------------------------------------------------------------------------------

def files_page(d, name, x, y, view="My files", light=False):
    f = v2.page(d, name, x, y, light=light, nav="Files", footer_left=FOOT_LEFT)
    sidebar(d, f, view)
    return f


def sidebar(d, f, view):
    sb = d.frame(f, "Sidebar", 0, 48, SIDE, H - 48 - 34, fill="bg", clip=True)
    side_stroke(d, sb, right=1)
    items = vstack(d, sb, "Views", 12, 24, SIDE - 25, gap=2)
    for key, label in (("folder", "My files"), ("clock", "Recent"), ("star", "Starred"), ("trash", "Trash")):
        on = label == view
        it = d.frame(items, f"View {label}", 0, 0, SIDE - 25, 36, fill="invert_bg" if on else None,
                     layout="horizontal", padding=[0, 12, 0, 13], gap=13, crossAlign="center", sizingH="fill",
                     sizingV="fixed")
        ink = "invert_fg" if on else "ink"
        if key == "star":
            star(d, it, ink, 14, outline=True)
        else:
            glyph(d, it, key, ink)
        hud(d, it, label, color=ink, name="View label")
    st = vstack(d, sb, "Storage", 20, H - 48 - 34 - 16 - 70, SIDE - 41, gap=6)
    hud(d, st, "Storage", color="muted", name="Storage title")
    r = hstack(d, st, "Used", gap=6)
    d.text(r, "12.4 GB", "Body/16 Medium", "ink_strong", name="Used")
    d.text(r, "of 100 GB", "Body/13", "muted", name="Quota")
    tr = d.frame(st, "Meter", 0, 0, SIDE - 41, 4, fill="line", sizingH="fill")
    d.rect(tr, 0, 0, round((SIDE - 41) * 0.124), 4, "accent", name="Fill")
    return sb


def toolbar(d, f, view="My files", names=CRUMBS, query=None, w=MW, x=X0, search_w=240):
    """files.rs toolbar_ui: where we are on the left; search, refresh, grid/list and the actions on the right."""
    tb = d.frame(f, "Toolbar", x, TOOL_Y, w, 34, fill=None, layout="horizontal", gap=8, crossAlign="center",
                 clip=False)
    crumbs(d, tb, names, own_case_from=1 if view == "My files" else 9)
    d.frame(tb, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    if view == "Trash":
        search(d, tb, "Filter the trash", w=search_w)
    elif query:
        search(d, tb, query, w=search_w, typed=True)
    else:
        search(d, tb, "Search files", w=search_w)
    square_btn(d, tb, "refresh", name="Refresh")
    square_btn(d, tb, "grid", name="Show as grid")
    if view == "My files":
        ib(d, tb, "New folder", "plus", "Secondary")
        ib(d, tb, "Upload", "up", "Primary")
    if view == "Trash":
        btn(d, tb, "Empty trash", "Danger")
    return tb


def meta_line(d, f, text, x=X0, w=MW, selected=None, trash=False):
    r = d.frame(f, "Meta", x, META_Y, w, 28, fill=None, layout="horizontal", gap=10, crossAlign="center",
                clip=False)
    if selected:
        hud(d, r, f"{selected} selected", color="ink_strong", name="Selected")
        acts = ("Restore", "Delete forever") if trash else ("Download", "Star", "Move", "Delete")
        for a in acts + ("Clear",):
            hud(d, r, a, color="bad" if a.startswith("Delete") else "accent_ink", name=f"Action {a}")
    else:
        hud(d, r, text, color="muted", name="Counts")
    return r


# --- the table ------------------------------------------------------------------------------------------

def columns(view, width):
    """table.rs columns(): (key, width); the name takes what is left."""
    cols = [("check", CHECK_W), ("Name", None)]
    if width >= 900:
        cols.append(("Opens in", 170) if view == "My files" else ("Location", 200))
    if view == "Trash":
        cols.append(("Deleted", 160 if width >= 640 else 132))
        if width >= 760:
            cols.append(("Deleted for good", 196))
        if width >= 640:
            cols.append(("Size", 96))
    else:
        if width >= 640:
            cols.append(("Opened", 150) if view == "Recent" else ("Modified", 150))
        if width >= 420:
            cols.append(("Size", 96))
    cols.append(("more", MORE_W))
    fixed = sum(w for _, w in cols if w)
    return [(k, w if w else max(120, width - fixed)) for k, w in cols]


def table(d, f, x, y, width, view, rows, sort=("Name", True), selected=(), hover=None, light=False, all_on=False):
    """rows: dicts with name, app, cells {col: text}, starred, shared."""
    cols = columns(view, width)
    t = d.frame(f, "Table", x, y, width, HEAD_H + ROW_H * len(rows), fill=None, layout="vertical", clip=False)
    hd = d.frame(t, "Header row", 0, 0, width, HEAD_H, fill=None, layout="horizontal", crossAlign="center",
                 sizingH="fill")
    side_stroke(d, hd, top=1, bottom=1)
    for key, cw in cols:
        c = d.frame(hd, f"H {key}", 0, 0, cw, HEAD_H, fill=None, layout="horizontal", gap=6, crossAlign="center",
                    mainAlign="center" if key == "check" else "min", sizingV="fill",
                    padding=[0, 0, 0, 34] if key == "Name" else [0, 0, 0, 0])
        if key == "check":
            check(d, c, on=all_on)
        elif key != "more":
            on = key == sort[0]
            hud(d, c, key, color="ink_strong" if on else "muted", name="Label")
            if on:
                a = d.inst("Icon/Sort up", c, name="Sort arrow")
                d.set(d.sub(a, "glyph"), fill="ink_strong")
                if not sort[1]:
                    d.set(a, rotation=180)
    for r in rows:
        nm = r["name"]
        sel = nm in selected
        rw = d.frame(t, f"Row {nm}", 0, 0, width, ROW_H, fill=SEL_WASH[light] if sel else
                     ("sunken" if nm == hover else None), layout="horizontal", crossAlign="center", sizingH="fill")
        side_stroke(d, rw, bottom=1)
        if sel:
            bar = d.rect(rw, 0, 0, 2, ROW_H, "accent", name="Selected rule")
            d.absolute(bar, 0, 0)
        for key, cw in cols:
            c = d.frame(rw, f"{key} cell", 0, 0, cw, ROW_H, fill=None, layout="horizontal", gap=12,
                        crossAlign="center", mainAlign="center" if key in ("check", "more") else "min",
                        sizingV="fill", clip=True, padding=[0, 12, 0, 2] if key == "Name" else [0, 0, 0, 0])
            if key == "check":
                check(d, c, on=sel, strong=nm == hover or bool(selected))
            elif key == "Name":
                file_icon(d, c, r["app"])
                d.text(c, nm, "Body/14", "ink_strong", name="Name")
                d.frame(c, "Spacer", 0, 0, 4, 4, fill=None, sizingH="fill")
                if r.get("starred"):
                    star(d, c, "warn", 13)
                if r.get("shared"):
                    glyph(d, c, "link", "muted")
            elif key == "Opens in":
                a = r["app"]
                if a and a != "folder":
                    app_chip(d, c, a)
                else:
                    d.text(c, "—" if a == "folder" else "Download", "Body/13", "muted", name="Opens")
            elif key == "more":
                more_dots(d, c, "ink_strong" if sel or nm == hover else "muted")
            else:
                txt = r["cells"].get(key, "")
                tt = d.text(c, txt, "Mono/12" if key == "Size" else "Body/13", "muted", name=key)
                if key == "Location":  # table.rs fit_text: cut with "…" 16 px before the next column
                    d.set(c, padding=[0, 16, 0, 0])
                    d.set(tt, sizingH="fill", textAutoResize="truncate", maxLines=1)
    return t


def folder_rows(names, view="My files", extra=None):
    out = []
    for n in names:
        nm, app, typ, mod, size, starred, shared, loc = BY_NAME[n]
        cells = {"Modified": mod, "Size": size, "Location": loc, "Opened": mod}
        if extra and n in extra:
            cells.update(extra[n])
        out.append(dict(name=nm, app=app, starred=starred, shared=shared, cells=cells))
    return out


ALL = [f[0] for f in FILES]


def browser(d, f, light=False, selected=(), hover=None, width=MW, meta=True, names=CRUMBS, search_w=240):
    toolbar(d, f, w=width, names=names, search_w=search_w)
    if meta:
        meta_line(d, f, META, w=width, selected=len(selected) if selected else None)
    return table(d, f, X0, LIST_Y, width, "My files", folder_rows(ALL), selected=selected, hover=hover, light=light)


def node_box(d, nid):
    b = d.R("node.get", {"id": nid, "fields": ["absoluteBounds"]})["absoluteBounds"]
    return b


def rel(d, f, nid):
    a, b = node_box(d, nid), node_box(d, f)
    return a["x"] - b["x"], a["y"] - b["y"], a["width"], a["height"]


# --- frames: the browser ---------------------------------------------------------------------------------

def v2_files(d, name, x, y, light=False):
    f = files_page(d, name, x, y, light=light)
    browser(d, f, light=light, selected=("Q3 report.docx",), hover="Poster.psd")
    return f


def v2_files_empty(d, name, x, y):
    f = files_page(d, name, x, y)
    toolbar(d, f, names=("My files", "Documents", "Drafts"))
    meta_line(d, f, "0 folders · 0 files")
    note = vstack(d, f, "Empty note", X0 + (MW - 440) / 2, LIST_Y - 8 + 130 - 90, 440, gap=6, crossAlign="center")
    d.text(note, "This folder is empty", "Display/Dialog title 22", "ink_strong", name="Empty title")
    d.text(note, "Files you add here sync to every computer you connect.", "Body/14", "muted", name="Empty note")
    d.frame(note, "Gap", 0, 0, 10, 8, fill=None)
    ib(d, note, "Upload files", "up", "Secondary")
    return f


def v2_files_loading(d, name, x, y):
    f = files_page(d, name, x, y)
    toolbar(d, f, names=("My files", "Documents", "Reports"))
    t = hud(d, f, "Loading…", color="muted", name="Loading")
    w = d.R("node.get", {"id": t, "fields": ["width"]})["width"]
    d.set(t, x=X0 + (MW - w) / 2, y=LIST_Y + 80)
    return f


def v2_files_details(d, name, x, y):
    """New in 0.2.0: the details panel beside the list, for the one selected file."""
    f = files_page(d, name, x, y)
    dw = 360
    lw = W - X0 - PAD - dw
    browser(d, f, selected=("Q3 report.docx",), width=lw, names=("…", "Reports"), search_w=200)
    details_panel(d, f, W - dw, dw)
    return f


def details_panel(d, f, x, w, light=False):
    p = d.frame(f, "Details", x, 48, w, H - 48 - 34, fill="bg", layout="vertical", padding=[20, 24, 24, 24], gap=16,
                clip=True)
    side_stroke(d, p, left=1)
    head = hstack(d, p, "Details head", gap=8)
    d.set(head, sizingH="fill")
    hud(d, head, "Details", color="muted", name="Details title")
    new_tag(d, head)
    d.frame(head, "Spacer", 0, 0, 4, 4, fill=None, sizingH="fill")
    glyph(d, head, "x", "muted", name="Close")
    pv = d.frame(p, "Preview", 0, 0, w - 48, 148, fill="sunken", stroke="line", layout="horizontal",
                 mainAlign="center", crossAlign="center", sizingH="fill")
    ic = d.inst("App icon", pv, name="App icon")
    d.R("image.replace", {"ids": [d.sub(ic, "Icon image")], "path": ICON("wordcraft")})
    col = vstack(d, p, "Name block", 0, 0, w - 48, gap=4)
    d.set(col, sizingH="fill")
    nr = hstack(d, col, "Name row", gap=10)
    d.set(nr, sizingH="fill")
    t = d.text(nr, "Q3 report.docx", "Display/Dialog title 22", "ink_strong", name="Detail name")
    d.set(t, sizingH="fill")
    sb = d.frame(nr, "Star button", 0, 0, 34, 34, fill=None, stroke="line_strong", layout="horizontal",
                 mainAlign="center", crossAlign="center")
    star(d, sb, "ink", 14, outline=True)
    d.text(col, f"{KIND['wordcraft']} · 248 KB · opens in {APP['wordcraft']['name']}", "Small/12.5", "muted",
           name="Detail kind")
    o = ib(d, p, f"Open in {APP['wordcraft']['name']}", "arrow", "Primary", name="Open in app")
    d.set(o, sizingH="fill", mainAlign="center")
    ac = hstack(d, p, "Detail actions", gap=8)
    d.set(ac, sizingH="fill")
    for lab, key in (("Download", "download"), ("Share…", "share")):
        b = ib(d, ac, lab, key, "Secondary")
        d.set(b, sizingH="fill", mainAlign="center")
    mb = d.frame(ac, "More", 0, 0, 34, 34, fill="bg", stroke="line_strong", layout="horizontal", mainAlign="center",
                 crossAlign="center")
    more_dots(d, mb, "ink")
    facts = v2.panel(d, p, "About this file", w - 48, gap=6)
    for k, val in (("Modified", "Today 11:42"), ("Location", "My files/Documents/Reports"), ("Owner", OWNER[1]),
                   ("Shared", "Link · view only")):
        r = hstack(d, facts, f"Fact {k}", gap=0)
        d.set(r, sizingH="fill")
        lab = hud(d, r, k, name=f"{k} key")
        d.set(lab, sizingH="fixed", width=92)
        v = d.text(r, val, "Body/13", "ink", name=f"{k} value")
        d.set(v, sizingH="fill", textAutoResize="truncate", maxLines=1)
    vs = v2.panel(d, p, "Versions · last 5 kept", w - 48, gap=0)
    for i, (when, size) in enumerate(VERSIONS[:3]):
        r = d.frame(vs, f"Version {i}", 0, 0, w - 76, 32, fill=None, layout="horizontal", gap=8,
                    crossAlign="center", sizingH="fill", sizingV="fixed")
        if i:
            side_stroke(d, r, top=1)
        d.text(r, when, "Body/13 Medium" if i == 0 else "Body/13", "ink", name="When")
        d.text(r, size, "Mono/12", "muted", name="Size")
        d.frame(r, "Spacer", 0, 0, 4, 4, fill=None, sizingH="fill")
        if i == 0:
            d.inst("Status badge", r, props={"Tone": "Good", "Label": "Current"}, name="Current")
        else:
            hud(d, r, "Restore", color="accent_ink", name="Restore")
    hud(d, vs, "All versions…", color="accent_ink", name="All versions")
    return p


def v2_files_menu(d, name, x, y):
    f = files_page(d, name, x, y)
    t = browser(d, f, selected=("Contract.pdf",))
    rx, ry, rw_, rh = rel(d, f, d.sub(t, "Row Contract.pdf"))
    wm = 240
    mx, my = X0 + MW - wm + 8, ry + rh
    wc = APP["pdfcraft"]["name"]
    m = menu(d, f, mx, my, [(f"Open in {wc}", "pdfcraft"), ("Download",), ("Share…",), ("Versions…",), "-",
                            ("Rename…",), ("Move to…",), ("Star", None, None, "hover"), "-",
                            ("Move to trash", None, None, "danger")], w=wm, name="Row menu")
    for item in (f"Open in {wc}", "Share…", "Versions…", "Move to trash"):
        _, iy, _, ih = rel(d, f, d.sub(m, f"Item {item}"))
        tag = new_tag(d, f, text="New in 0.2.0" if item != "Move to trash" else "0.2.0: was “Delete”",
                      name=f"Tag {item}")
        tw = d.R("node.get", {"id": tag, "fields": ["width"]})["width"]
        d.set(tag, x=mx - tw - 10, y=iy + (ih - 18) / 2)
    return f


def v2_files_selected(d, name, x, y):
    """Two picked: the meta line turns into the selection's actions; a delete's notice offers UNDO."""
    f = files_page(d, name, x, y)
    browser(d, f, selected=("Logo.svg", "Poster.psd"))
    toast_undo(d, f, X0 + MW / 2, "Moved “archive.zip” to the trash")
    return f


def toast_undo(d, f, cx, text, bottom=H - 34):
    """files.rs toast_ui: 40 tall, bottom centre, invert_bg, 13.5 text, UNDO in mono 11."""
    t = d.frame(f, "Toast", 0, 0, 10, 40, fill="invert_bg", layout="horizontal", padding=[0, 0, 0, 20], gap=20,
                crossAlign="center", sizingH="hug", sizingV="fixed", clip=False)
    d.effect_style(t, "Shadow/Dialog")
    d.text(t, text, "Body/13.5 Tagline", "invert_fg", name="Toast text")
    u = d.frame(t, "Undo", 0, 0, 64, 40, fill=None, layout="horizontal", mainAlign="center", crossAlign="center")
    d.text(u, "UNDO", "Mono/12", "accent", name="Undo label")
    w = d.R("node.get", {"id": t, "fields": ["width"]})["width"]
    d.set(t, x=cx - w / 2, y=bottom - 36 - 20)
    return t


# --- frames: upload -------------------------------------------------------------------------------------

def v2_files_drop(d, name, x, y):
    f = files_page(d, name, x, y)
    browser(d, f)
    lh = H - 34 - LIST_Y
    ov = d.frame(f, "Drop overlay", X0 + 2, LIST_Y + 2, MW - 4, lh - 4, fill=DROP_WASH)
    d.set(ov, strokes=[{"type": "solid", "color": "#5e7a48"}], strokeWeight=2, strokeAlign="inside")
    d.R("variable.bind", {"ids": [ov], "field": "strokes/0/color", "variable": "accent"})
    t = d.text(ov, "DROP TO UPLOAD TO REPORTS", "Mono/13", "ink_strong", name="Drop label")
    b = d.R("node.get", {"id": t, "fields": ["width", "height"]})
    d.set(t, x=(MW - 4 - b["width"]) / 2, y=(lh - 4 - b["height"]) / 2)
    return f


UPLOADS = [("Q4 plan.docx", "Done", "good", 1.0), ("Moodboard.png", "3.1 MB of 6.4 MB", "muted", 0.48),
           ("Ideas.md", "Name taken", "warn", 0.0), ("notes.md", "Waiting", "muted", 0.0)]


def uploads_panel(d, f, items, title, area_right=W, bottom=H - 34, m=PAD, w=360):
    h = 44 + 46 * len(items)
    p = d.frame(f, "Uploads", area_right - w - m, bottom - h - m, w, h, fill="bg", stroke="line_strong",
                layout="vertical", clip=True)
    d.effect_style(p, "Shadow/Dialog")
    hd = d.frame(p, "Uploads head", 0, 0, w, 44, fill=None, layout="horizontal", padding=[0, 14, 0, 14],
                 crossAlign="center", sizingH="fill")
    side_stroke(d, hd, bottom=1)
    hud(d, hd, title, color="ink_strong", name="Uploads title")
    d.frame(hd, "Spacer", 0, 0, 4, 4, fill=None, sizingH="fill")
    glyph(d, hd, "x", "ink", name="Close uploads")
    for nm, status, tone, frac in items:
        r = d.frame(p, f"Upload {nm}", 0, 0, w, 46, fill=None, clip=False)
        d.text(r, nm, "Body/13", "ink_strong", x=14, y=9, name="Upload name")
        s = d.text(r, status.upper(), "Badge/10", tone, name="Upload status")
        sw = d.R("node.get", {"id": s, "fields": ["width"]})["width"]
        d.set(s, x=w - 14 - sw, y=10)
        d.rect(r, 14, 46 - 12, w - 28, 3, "line", name="Track")
        if frac:
            d.rect(r, 14, 46 - 12, round((w - 28) * frac), 3, "accent", name="Progress")
    return p


def v2_files_upload(d, name, x, y):
    f = files_page(d, name, x, y)
    browser(d, f)
    uploads_panel(d, f, UPLOADS, "Uploading 2 of 4")
    return f


def v2_files_upload_taken(d, name, x, y):
    f = files_page(d, name, x, y)
    browser(d, f)
    v2.scrim(d, f, W, H)
    sw, sh = 560, 320
    s, body = v2.sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "This name is taken", "Upload")
    d.set(body, gap=4)
    d.text(body, "“Ideas.md” is already in Reports.", "Body/16 Medium", "ink_strong", name="Clash line", w=sw - 48)
    d.text(body, "Replace keeps the current file as an earlier version, so nothing is lost. Keep both adds a number "
                 "to the new one.", "Body/13", "muted", name="Clash note", w=sw - 48)
    d.frame(body, "Gap", 0, 0, 10, 8, fill=None)
    cr = hstack(d, body, "Every name", gap=8)
    d.inst("Checkbox", cr, props={"State": "Off"}, name="Every checkbox")
    d.text(cr, "Do this for every name that is taken", "Body/13", "ink", name="Every label")
    v2.spacer(d, body, v=True)
    v2.sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Skip"),
                     right=lambda r: (btn(d, r, "Keep both"), btn(d, r, "Replace", "Primary")))
    uploads_panel(d, f, UPLOADS[2:], "Uploading 1 of 2")
    return f


# --- frames: sheets over the browser ---------------------------------------------------------------------

def over_browser(d, name, x, y, selected=()):
    f = files_page(d, name, x, y)
    browser(d, f, selected=selected)
    v2.scrim(d, f, W, H)
    return f


def keep_case(d, s):
    try:
        d.set(d.sub(s, "Sheet caption"), textCase="original")
    except Exception as e:  # noqa: BLE001  (a FigCraft refusal leaves the caption in caps)
        print("note: caption case:", e)


def name_sheet(d, name, x, y, title, caption, value, primary, selected=()):
    f = over_browser(d, name, x, y, selected=selected)
    sw, sh = 480, 300
    s, body = v2.sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, title, caption)
    keep_case(d, s)
    v2.field(d, body, "Name", value, state="Focus", w=sw - 48)
    v2.spacer(d, body, v=True)
    v2.sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"), right=lambda r: ib(d, r, primary, "check"))
    return f


def v2_files_new_folder(d, name, x, y):
    return name_sheet(d, name, x, y, "New folder", "Reports", "Figures 2026", "Create")


def v2_files_rename(d, name, x, y):
    return name_sheet(d, name, x, y, "Rename file", "Q3 report.docx", "Q3 report final.docx", "Rename",
                      selected=("Q3 report.docx",))


def v2_files_move(d, name, x, y):
    f = over_browser(d, name, x, y, selected=("Q3 report.docx",))
    sw, sh = 560, 560
    s, body = v2.sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "Move “Q3 report.docx”", "Choose a folder")
    d.set(body, gap=10)
    crumbs(d, body, ("My files", "Documents"))
    lst = vstack(d, body, "Folders", 0, 0, sw - 48, gap=0)
    d.set(lst, sizingH="fill")
    for nm in ("Archive", "Contracts", "Meetings", "Reports"):
        r = d.frame(lst, f"Folder {nm}", 0, 0, sw - 48, 42, fill="sunken" if nm == "Contracts" else None,
                    layout="horizontal", padding=[0, 12, 0, 12], gap=12, crossAlign="center", sizingH="fill")
        side_stroke(d, r, bottom=1)
        glyph(d, r, "folder", "ink")
        d.text(r, nm, "Body/14", "ink_strong" if nm == "Contracts" else "ink", name="Folder name")
    v2.spacer(d, body, v=True)
    v2.sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"), right=lambda r: ib(d, r, "Move here", "arrow"))
    return f


def v2_files_versions(d, name, x, y):
    """New in 0.2.0: the kept versions of a file (.craftcloud/versions/<path>/, the last 5), with restore."""
    f = over_browser(d, name, x, y, selected=("Q3 report.docx",))
    sw, sh = 720, 600
    s, body = v2.sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "Versions of Q3 report.docx",
                       "My files/Documents/Reports", icon="wordcraft", right=lambda hd: new_tag(d, hd))
    keep_case(d, s)
    d.text(body, "Before CraftCloud saves over a file, from a computer or from the web, it keeps the old one. "
                 "The last 5 are kept. Restoring keeps the current file as a version too, so nothing is lost.",
           "Body/14", "muted", name="Versions note", w=sw - 48)
    tbl = d.frame(body, "Versions table", 0, 0, sw - 48, 10, stroke="line", layout="vertical", sizingV="hug",
                  sizingH="fill", clip=False)
    hd = d.frame(tbl, "Head", 0, 0, sw - 48, 36, fill=None, layout="horizontal", padding=[0, 14, 0, 14],
                 crossAlign="center", sizingH="fill")
    side_stroke(d, hd, bottom=1)
    for lab, cw in (("Saved", 260), ("Size", 120)):
        c = d.frame(hd, f"H {lab}", 0, 0, cw, 36, fill=None, layout="horizontal", crossAlign="center")
        hud(d, c, lab, color="ink_strong" if lab == "Saved" else "muted", name="Label")
    for i, (when, size) in enumerate(VERSIONS):
        r = d.frame(tbl, f"Version {i}", 0, 0, sw - 48, 48, fill="sunken" if i == 2 else None, layout="horizontal",
                    padding=[0, 14, 0, 14], gap=8, crossAlign="center", sizingH="fill")
        if i < len(VERSIONS) - 1:
            side_stroke(d, r, bottom=1)
        c = d.frame(r, "When", 0, 0, 252, 48, fill=None, layout="horizontal", gap=8, crossAlign="center")
        d.text(c, when, "Body/14 Medium" if i == 0 else "Body/14", "ink", name="Time")
        if i == 0:
            d.inst("Status badge", c, props={"Tone": "Good", "Label": "Current"}, name="Current")
        c = d.frame(r, "Size", 0, 0, 112, 48, fill=None, layout="horizontal", crossAlign="center")
        d.text(c, size, "Mono/12", "muted", name="Bytes")
        d.frame(r, "Spacer", 0, 0, 4, 4, fill=None, sizingH="fill")
        if i == 0:
            hud(d, r, "Download", color="accent_ink", name="Download")
        else:
            hud(d, r, "Download", color="accent_ink", name="Download")
            btn(d, r, "Restore", state="Hover" if i == 2 else "Default")
    v2.spacer(d, body, v=True)
    v2.sheet_actions(d, s, sw, right=lambda r: btn(d, r, "Close", "Primary"))
    return f


# --- frames: trash --------------------------------------------------------------------------------------

TRASH = [  # name, app, location, deleted, deleted for good, size
    ("Old budget.xlsx", "gridcraft", "My files/Finance", "Today 09:12", "13 Dec 2026", "71 KB"),
    ("Moodboard v1.psd", "photocraft", "My files/Design", "Yesterday 15:40", "12 Dec 2026", "88.2 MB"),
    ("Notes.md", "wordcraft", "My files/Documents", "Yesterday 08:03", "12 Dec 2026", "2.8 KB"),
    ("Interview.wav", "soundcraft", "My files/Audio", "10 Nov 2026 16:12", "10 Dec 2026", "640 MB"),
    ("Drafts 2025", "folder", "My files/Documents", "8 Nov 2026 11:30", "8 Dec 2026", "—"),
    ("logo-old.svg", "vectorcraft", "My files/Design", "2 Nov 2026 09:41", "2 Dec 2026", "12 KB"),
    ("scan_0042.pdf", "pdfcraft", "My files/Inbox", "28 Oct 2026 14:05", "27 Nov 2026", "3.1 MB"),
]


def trash_rows():
    return [dict(name=n, app=a, cells={"Location": loc, "Deleted": dl, "Deleted for good": gone, "Size": sz})
            for n, a, loc, dl, gone, sz in TRASH]


def trash_page(d, name, x, y, menu_on=None, selected=()):
    f = files_page(d, name, x, y, view="Trash")
    toolbar(d, f, view="Trash", names=("Trash",))
    meta_line(d, f, "7 items · 732 MB · deleted for good after 30 days", selected=len(selected) or None,
              trash=True)
    t = table(d, f, X0, LIST_Y, MW, "Trash", trash_rows(), sort=("Deleted for good", True), selected=selected,
              hover=menu_on)
    if menu_on:
        _, ry, _, rh = rel(d, f, d.sub(t, f"Row {menu_on}"))
        menu(d, f, X0 + MW - 240 + 8, ry + rh, [("Restore", None, None, "hover"), ("Delete forever", None, None,
                                                                                     "danger")],
             w=240, name="Row menu")
    return f


def v2_files_trash(d, name, x, y):
    f = trash_page(d, name, x, y, menu_on="Moodboard v1.psd")
    return f


def v2_files_trash_delete(d, name, x, y):
    f = trash_page(d, name, x, y, selected=("Interview.wav",))
    v2.scrim(d, f, W, H)
    sw, sh = 520, 280
    s, body = v2.sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "Delete 1 item for good?", "Trash")
    d.text(body, "They can't be restored afterwards, and computers that sync them lose them too.", "Body/14",
           "muted", name="Body", w=sw - 48)
    v2.spacer(d, body, v=True)
    v2.sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"), right=lambda r: btn(d, r, "Delete forever",
                                                                                         "Danger"))
    return f


def v2_files_trash_taken(d, name, x, y):
    f = trash_page(d, name, x, y, selected=("Notes.md",))
    v2.scrim(d, f, W, H)
    sw, sh = 560, 320
    s, body = v2.sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "“Notes.md” is already there",
                       "Documents/Notes.md")
    keep_case(d, s)
    d.text(body, "Something called “Notes.md” is in its old place now. Restore it next to that as "
                 "“Notes (restored).md”, or leave it in the trash.", "Body/14", "muted", name="Body", w=sw - 48)
    v2.spacer(d, body, v=True)
    v2.sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"),
                     right=lambda r: btn(d, r, "Restore with the new name", "Primary"))
    return f


# --- frames: views --------------------------------------------------------------------------------------

def v2_files_recent(d, name, x, y):
    f = files_page(d, name, x, y, view="Recent")
    toolbar(d, f, view="Recent", names=("Recent",))
    opened = {"Q3 report.docx": "Today 11:42", "Poster.psd": "Today 11:20", "Logo.svg": "Today 10:02",
              "Contract.pdf": "Today 08:30", "Notes.md": "Today 07:55", "Budget 2027.xlsx": "Yesterday 16:05",
              "Floorplan.dwg": "11 Nov 2026", "Pitch.pptx": "10 Nov 2026"}
    meta_line(d, f, "0 folders · 8 files · 53.0 MB")
    rows = folder_rows(list(opened), extra={k: {"Opened": v} for k, v in opened.items()})
    table(d, f, X0, LIST_Y, MW, "Recent", rows, sort=("Opened", False), hover="Logo.svg")
    return f


def v2_files_starred(d, name, x, y):
    f = files_page(d, name, x, y, view="Starred")
    toolbar(d, f, view="Starred", names=("Starred",))
    meta_line(d, f, "1 folder · 2 files · 100 KB")
    table(d, f, X0, LIST_Y, MW, "Starred", folder_rows(["Figures", "Budget 2027.xlsx", "Logo.svg"]))
    return f


def v2_files_search(d, name, x, y):
    f = files_page(d, name, x, y, view="")
    toolbar(d, f, view="Search", names=("Results for “report”",), query="report")
    meta_line(d, f, "1 folder · 4 files · 5.1 MB")
    found = [("report-photos", "folder", "My files/Photos/2026", "Yesterday 18:20", "—"),
             ("Q3 report.docx", "wordcraft", "My files/Documents/Reports", "Today 11:42", "248 KB"),
             ("Q2 report.docx", "wordcraft", "My files/Documents/Reports/2026", "2 Jul 2026", "231 KB"),
             ("Annual report 2025.pdf", "pdfcraft", "My files/Documents/Archive", "14 Mar 2026", "4.6 MB"),
             ("Expense report.xlsx", "gridcraft", "My files/Finance", "31 Oct 2026", "64 KB")]
    rows = [dict(name=n, app=a, cells={"Location": loc, "Modified": m, "Size": s}) for n, a, loc, m, s in found]
    t = table(d, f, X0, LIST_Y, MW, "Search", rows, hover="Q2 report.docx")
    _, ry, _, rh = rel(d, f, d.sub(t, "Row Q2 report.docx"))
    menu(d, f, X0 + MW - 240 + 8, ry + rh, [("Download", None, None, "hover"), ("Show in folder",), "-",
                                             ("Rename…",), ("Move to…",), ("Star",), "-",
                                             ("Delete", None, None, "danger")], w=240, name="Row menu")
    return f


# --- frames: phone --------------------------------------------------------------------------------------

def v2_files_phone(d, name, x, y, light=False):
    """Under 900 px the sidebar becomes the view cells; under 600 the margin is 16, the breadcrumbs get their
    own row, Upload and New folder are squares, and a 358 px list keeps the name only."""
    f = screen(d, name, x, y, PW, PH, light=light)
    hd = d.frame(f, "Header", 0, 0, PW, 48, fill="bg", layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1)

    def cell(nm, pad=(0, 12, 0, 12), right=False, left=False, fixed=None):
        c = d.frame(hd, nm, 0, 0, fixed or 10, 48, fill=None, layout="horizontal", padding=list(pad), gap=4,
                    crossAlign="center", sizingH="fixed" if fixed else "hug", sizingV="fill", clip=False)
        if fixed:
            d.set(c, mainAlign="center")
        if left or right:
            side_stroke(d, c, left=1 if left else 0, right=1 if right else 0)
        return c

    b = cell("Brand", (0, 12, 0, 14), right=True)
    v2.logo(d, b, 24, name="Logo")
    n = cell("Nav", (0, 8, 0, 8), right=True)
    for lab in ("Home", "Files", "Admin"):
        d.inst("Nav cell", n, props={"Label": lab, "State": "On" if lab == "Files" else "Off"}, name=f"Nav {lab}")
    d.frame(hd, "Spacer", 0, 0, 4, 48, fill=None, sizingH="fill")
    u = cell("User", left=True)
    v2.avatar(d, u, OWNER[0])
    so = cell("Sign out", (0, 0, 0, 0), left=True, fixed=48)
    glyph(d, so, "logout", "ink")
    ft = d.frame(f, "Footer", 0, PH - 34, PW, 34, fill="bg", layout="horizontal", padding=[0, 16, 0, 16],
                 mainAlign="max", crossAlign="center")
    side_stroke(d, ft, top=1)
    d.text(ft, "Look based on ArtCraft Launcher by the ArtCraft team", "Small/12", "muted", name="Credit")

    pad, iw = 16, PW - 32
    yy = 48 + 16
    vc = v2.cells(d, f, ("My files", "Recent", "Starred", "Trash"), sel="My files", name="Views")
    d.set(vc, x=pad, y=yy)
    yy += 34 + 12
    tb = d.frame(f, "Toolbar", pad, yy, iw, 34, fill=None, layout="horizontal", gap=8, crossAlign="center",
                 clip=False)
    s = search(d, tb, "Search files", w=100)
    d.set(s, sizingH="fill")
    square_btn(d, tb, "refresh", name="Refresh")
    square_btn(d, tb, "grid", name="Show as grid")
    square_btn(d, tb, "plus", name="New folder")
    square_btn(d, tb, "up", on=True, name="Upload")
    yy += 34 + 14 - 4
    cr = crumbs(d, f, CRUMBS)
    d.set(cr, x=pad, y=yy)
    yy += 34 + 10
    m = d.frame(f, "Meta", pad, yy, iw, 28, fill=None, layout="horizontal", crossAlign="center", clip=False)
    hud(d, m, META, color="muted", name="Counts")
    yy += 28 + 8
    names = ["Drafts", "Figures", "Budget 2027.xlsx", "Contract.pdf", "Logo.svg", "Notes.md", "Pitch.pptx",
             "Poster.psd", "Q3 report.docx", "Theme.wav"]
    table(d, f, pad, yy, iw, "My files", folder_rows(names), light=light, selected=("Q3 report.docx",))
    return f


# --- build ----------------------------------------------------------------------------------------------

def build(d):
    """Makes the page "v2 · Files" and its frames; returns [(export name, frame id)]."""
    d.page("v2 · Files")
    frames = []
    col, rowh = W + GAP, H + GAP
    web = [
        ("v2-web-files", lambda n, x, y: v2_files(d, n, x, y)),
        ("v2-web-files-light", lambda n, x, y: v2_files(d, n, x, y, light=True)),
        ("v2-web-files-empty", lambda n, x, y: v2_files_empty(d, n, x, y)),
        ("v2-web-files-loading", lambda n, x, y: v2_files_loading(d, n, x, y)),
        ("v2-web-files-details", lambda n, x, y: v2_files_details(d, n, x, y)),
        ("v2-web-files-menu", lambda n, x, y: v2_files_menu(d, n, x, y)),
        ("v2-web-files-selected", lambda n, x, y: v2_files_selected(d, n, x, y)),
        ("v2-web-files-drop", lambda n, x, y: v2_files_drop(d, n, x, y)),
        ("v2-web-files-upload", lambda n, x, y: v2_files_upload(d, n, x, y)),
        ("v2-web-files-upload-taken", lambda n, x, y: v2_files_upload_taken(d, n, x, y)),
        ("v2-web-files-new-folder", lambda n, x, y: v2_files_new_folder(d, n, x, y)),
        ("v2-web-files-rename", lambda n, x, y: v2_files_rename(d, n, x, y)),
        ("v2-web-files-move", lambda n, x, y: v2_files_move(d, n, x, y)),
        ("v2-web-files-versions", lambda n, x, y: v2_files_versions(d, n, x, y)),
        ("v2-web-files-trash", lambda n, x, y: v2_files_trash(d, n, x, y)),
        ("v2-web-files-trash-delete", lambda n, x, y: v2_files_trash_delete(d, n, x, y)),
        ("v2-web-files-trash-taken", lambda n, x, y: v2_files_trash_taken(d, n, x, y)),
        ("v2-web-files-recent", lambda n, x, y: v2_files_recent(d, n, x, y)),
        ("v2-web-files-starred", lambda n, x, y: v2_files_starred(d, n, x, y)),
        ("v2-web-files-search", lambda n, x, y: v2_files_search(d, n, x, y)),
    ]
    for i, (n, fn) in enumerate(web):
        r, c = divmod(i, 4)
        frames.append((n, fn(n, c * col, r * rowh)))
    yph = (len(web) + 3) // 4 * rowh
    frames.append(("v2-web-files-phone", v2_files_phone(d, "v2-web-files-phone", 0, yph)))
    frames.append(("v2-web-files-phone-light", v2_files_phone(d, "v2-web-files-phone-light", PW + 120, yph,
                                                              light=True)))
    return frames
