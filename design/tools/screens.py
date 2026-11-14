"""Screens (one page per screen group) built from the component library."""

import os

from lib import APP, ASSETS, ORDER, PINNED, THEME, USER, STORAGE, VERSION, OXICLOUD, CATS
from cards import CARD_W, CARD_H, fill_card, fill_row, TABLE_COLS
from components import ICON

W, H = 1440, 900
GAP = 160  # between frames on a page
DETAILS_W = 360


def tokens_page(d):
    """Colour swatches next to the type specimens (both modes)."""
    d.set(d.specimens, x=0, y=0)
    for mi, mode in enumerate(("Dark", "Light")):
        f = d.frame(None, f"Colour tokens ({mode.lower()})", 820 + mi * 560, 0, 520, 40 + 44 * len(THEME), fill="bg")
        if mode == "Light":
            d.light([f])
        for i, k in enumerate(THEME):
            d.rect(f, 24, 24 + i * 44, 32, 32, k, name=k, stroke="line", strokeWeight=1, strokeAlign="inside")
            d.text(f, k, "Mono/13", "ink", x=72, y=30 + i * 44, name=f"{k} label")
            d.text(f, THEME[k][mi], "Mono/12", "muted", x=300, y=31 + i * 44, name=f"{k} value")


def side_stroke(d, ids, color="line", **sides):
    """Hairline on some sides only (top/right/bottom/left = 1)."""
    if not isinstance(ids, list):
        ids = [ids]
    w = {f"stroke{k.capitalize()}Weight": sides.get(k, 0) for k in ("top", "right", "bottom", "left")}
    d.set(ids, strokes=[{"type": "solid", "color": "#ffffff"}], strokeAlign="inside", **w)
    d.R("variable.bind", {"ids": ids, "field": "strokes/0/color", "variable": color})


# --- components that only screens use ------------------------------------------------------------

FILES = [
    # name, app (None = folder / download), modified, size, shared
    ("Drafts", "folder", "Today 09:10", "6 items", "—"),
    ("Budget 2027.xlsx", "gridcraft", "Yesterday 16:05", "86 KB", "2 people"),
    ("Contract.pdf", "pdfcraft", "Today 08:30", "1.2 MB", "Link"),
    ("Floorplan.dwg", "cadcraft", "11 Nov 2026", "3.4 MB", "—"),
    ("IMG_2041.CR3", "lightcraft", "9 Nov 2026", "28.6 MB", "—"),
    ("Logo.svg", "vectorcraft", "Today 10:02", "14 KB", "Link"),
    ("Notes.md", "wordcraft", "Today 07:55", "3 KB", "—"),
    ("Pitch.pptx", "deckcraft", "10 Nov 2026", "5.8 MB", "2 people"),
    ("Poster.psd", "photocraft", "Today 11:20", "42.1 MB", "—"),
    ("Q3 report.docx", "wordcraft", "Today 11:42", "248 KB", "Link"),
    ("Theme.wav", "soundcraft", "9 Nov 2026", "31.0 MB", "—"),
    ("Trailer.mp4", "filmcraft", "6 Nov 2026", "412 MB", "2 people"),
    ("archive.zip", None, "2 Nov 2026", "96.3 MB", "—"),
]
FILE_COLS = [("Name", None), ("Opens in", 180), ("Modified", 150), ("Size", 96), ("Shared", 104), ("", 44)]
VERSIONS = [
    ("Current", "Today 11:42", "248 KB", "WordCraft"),
    ("", "Today 10:15", "244 KB", "WordCraft"),
    ("", "Yesterday 17:30", "231 KB", "WordCraft"),
    ("", "Yesterday 09:12", "229 KB", "WordCraft"),
    ("", "11 Nov 2026 15:48", "210 KB", "WordCraft"),
    ("", "10 Nov 2026 11:03", "188 KB", "Uploaded"),
]


def more_components(d, lib):
    # Menu item: optional app icon, label, optional shortcut/chevron; hover and danger variants.
    y = lib.row(32)
    ids = []
    for i, state in enumerate(("Default", "Hover")):
        c = d.comp(f"State={state}", i * 280, y, 240, 32, fill="sunken" if state == "Hover" else None,
                   layout="horizontal", padding=[0, 12, 0, 12], gap=10, crossAlign="center", sizingV="fixed")
        ic = d.inst("App icon/small", c, name="Item icon")
        d.set(ic, width=16, height=16)
        t = d.text(c, "Open in WordCraft", "Body/13", "ink_strong" if state == "Hover" else "ink", name="Label")
        d.set(t, sizingH="fill")
        k = d.text(c, "›", "Body/13", "muted", name="Trail")
        d.prop(c, t, "Label")
        d.prop(c, k, "Trail")
        d.R("component.addProperty", {"id": c, "name": "Icon", "type": "boolean", "layer": ic, "default": True})
        d.R("component.addProperty", {"id": c, "name": "Show trail", "type": "boolean", "layer": k, "default": False})
        d.set([ic, k], visible=False) if False else None
        ids.append(c)
    s = d.R("component.combineVariants", {"ids": ids})["id"]
    d.set(s, name="Menu item", x=0, y=y)
    d.C["Menu item"] = s

    # File row (list view), 48 tall.
    y = lib.row(48)
    wlist = W - DETAILS_W - 48
    ids = []
    for i, state in enumerate(("Default", "Hover", "Selected")):
        c = d.comp(f"State={state}", 0, y + i * 70, wlist, 48, fill={"Default": "bg", "Hover": "sunken",
                                                                     "Selected": "sunken"}[state],
                   layout="horizontal", crossAlign="center", sizingV="fixed")
        side_stroke(d, c, bottom=1)
        if state == "Selected":
            bar = d.rect(c, 0, 0, 2, 48, "accent_ink", name="Selected rule")
            d.set(bar, absolute=True)
        name_cell = d.frame(c, "Name cell", 0, 0, 300, 48, fill=None, layout="horizontal", padding=[0, 12, 0, 16],
                            gap=12, crossAlign="center", sizingH="fill", sizingV="fill")
        fi = d.inst("Icon/File", name_cell, name="File icon")
        d.set(d.sub(fi, "page"), fill="app:wordcraft/color")
        fo = d.inst("Icon/Folder", name_cell, name="Folder icon")
        d.set(fo, visible=False)
        nm = d.text(name_cell, "Q3 report.docx", "Body/14 Medium" if state == "Selected" else "Body/14", "ink",
                    name="File name")
        cells = {}
        for lab, w in FILE_COLS[1:]:
            cells[lab] = d.frame(c, f"{lab or 'More'} cell", 0, 0, w, 48, fill=None, layout="horizontal",
                                 padding=[0, 12, 0, 12] if lab else [0, 0, 0, 0], gap=8, crossAlign="center",
                                 mainAlign="center" if not lab else "min", sizingH="fixed", sizingV="fill")
        ai = d.inst("App icon/small", cells["Opens in"], name="Opens icon")
        an = d.text(cells["Opens in"], "WordCraft", "Body/13", "ink", name="Opens app")
        md = d.text(cells["Modified"], "Today 11:42", "Body/13", "muted", name="Modified")
        sz = d.text(cells["Size"], "248 KB", "Mono/12", "muted", name="Size")
        sh = d.text(cells["Shared"], "Link", "Body/13", "muted", name="Shared")
        more = d.text(cells[""], "…", "Body/14 Medium", "muted" if state == "Default" else "ink", name="More")
        for lay, pn in ((nm, "Name"), (an, "App"), (md, "Modified"), (sz, "Size"), (sh, "Shared")):
            d.prop(c, lay, pn)
        d.R("component.addProperty", {"id": c, "name": "App icon", "type": "boolean", "layer": ai, "default": True})
        ids.append(c)
    s = d.R("component.combineVariants", {"ids": ids})["id"]
    d.set(s, name="File row", x=0, y=y)
    d.C["File row"] = s
    lib.y += 140

    # Editor bar, one variant per save state.
    y = lib.row(40)
    ids = []
    for i, st in enumerate(("Opening", "Unsaved", "Saving", "Saved", "Failed", "Read-only")):
        ids.append(editor_bar(d, st, 0, y + i * 70))
    s = d.R("component.combineVariants", {"ids": ids})["id"]
    d.set(s, name="Editor bar", x=0, y=y)
    d.C["Editor bar"] = s
    lib.y += 6 * 70


def editor_bar(d, state, x, y, app="wordcraft"):
    c = d.comp(f"State={state}", x, y, W, 40, fill="bg", layout="horizontal", crossAlign="center", sizingV="fixed",
               clip=True)
    side_stroke(d, c, bottom=1)
    back = d.frame(c, "Back", 0, 0, 40, 40, fill=None, layout="horizontal", mainAlign="center", crossAlign="center")
    side_stroke(d, back, right=1)
    d.text(back, "←", "Body/14", "ink", name="Back arrow")
    app_cell = d.frame(c, "App", 0, 0, 10, 40, fill=None, layout="horizontal", padding=[0, 16, 0, 16], gap=8,
                       crossAlign="center", sizingH="hug", sizingV="fill")
    side_stroke(d, app_cell, right=1)
    d.inst("App icon/small", app_cell, name="App icon")
    wm = d.inst("App wordmark/14", app_cell, name="Wordmark")
    crumbs = d.frame(c, "Path", 0, 0, 10, 40, fill=None, layout="horizontal", padding=[0, 16, 0, 16], gap=8,
                     crossAlign="center", sizingH="hug", sizingV="fill")
    for i, part in enumerate(("Documents", "Reports", "Q3 report.docx")):
        if i:
            d.text(crumbs, "›", "Body/13", "muted", name="sep")
        d.text(crumbs, part, "Body/13 Medium" if i == 2 else "Body/13", "ink" if i == 2 else "muted",
               name="File" if i == 2 else part)
    tone, label = {"Opening": ("Accent", "Opening…"), "Unsaved": ("Warn", "Unsaved changes"),
                   "Saving": ("Accent", "Saving…"), "Saved": ("Good", "Saved"), "Failed": ("Bad", "Save failed"),
                   "Read-only": ("Muted", "Read-only")}[state]
    d.inst("Status badge", crumbs, props={"Tone": tone, "Label": label}, name="Save state")
    if state == "Saved":
        d.text(crumbs, "11:42", "Small/12", "muted", name="Saved at")
    if state == "Read-only":
        d.inst("Icon/Lock", crumbs, name="Lock")
        d.text(crumbs, "You can view this file but not save over it. Save a copy instead.", "Small/12", "muted",
               name="Read-only note")
    if state == "Failed":
        d.text(crumbs, "The cloud didn't answer (HTTP 503). Your changes are still in the app.", "Small/12", "bad",
               name="Error")
    d.frame(c, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    right = d.frame(c, "Actions", 0, 0, 10, 40, fill=None, layout="horizontal", padding=[0, 12, 0, 12], gap=6,
                    crossAlign="center", sizingH="hug", sizingV="fill")
    if state == "Failed":
        b = d.inst("Button", right, props={"Kind": "Primary", "Label": "Retry"}, name="Retry")
        b2 = d.inst("Button", right, props={"Kind": "Secondary", "Label": "Download copy"}, name="Download copy")
        d.set([b, b2], height=28)
    if state == "Read-only":
        b = d.inst("Button", right, props={"Kind": "Secondary", "Label": "Save a copy…"}, name="Save copy")
        d.set(b, height=28)
    for lab in ("Versions", "Share"):
        t = d.frame(right, lab, 0, 0, 10, 28, fill=None, layout="horizontal", padding=[0, 10, 0, 10],
                    crossAlign="center", sizingH="hug", sizingV="fixed")
        d.text(t, lab, "Button/11", "ink", name=f"{lab} label")
    m = d.inst("Button/More", right, name="More")
    d.set(m, width=28, height=28)
    if state == "Opening":
        tr = d.rect(c, 0, 38, W, 2, "line", name="Progress track")
        d.absolute(tr, 0, 38)
        p = d.rect(c, 0, 38, round(W * 0.42), 2, f"app:{app}/color", name="Progress")
        d.absolute(p, 0, 38)
    return c


# --- shared screen parts ------------------------------------------------------------------------------

def screen(d, name, x, y, w=W, h=H, light=False):
    f = d.frame(None, name, x, y, w, h, fill="bg")
    if light:
        d.light([f])
    return f


def header(d, f, section="Apps", w=W):
    hid = d.inst("Header", f, 0, 0, name="Header")
    if w != W:
        d.set(hid, width=w)
    if section != "Apps":
        for lab in ("Apps", "Files", "Recent", "Shared", "Trash"):
            nid = d.sub(hid, f"Nav {lab}")
            d.R("instance.setProperties", {"ids": [nid], "properties": {"State": "On" if lab == section else "Off"}})
    return hid


def footer(d, f, h=H, w=W, left=None):
    fid = d.inst("Footer", f, 0, h - 34, name="Footer")
    if w != W:
        d.set(fid, width=w)
    if left:
        d.R("instance.setProperties", {"ids": [fid], "properties": {"Left": left}})
    return fid


def hud(d, parent, text, x=0, y=0, color="muted", name=None):
    return d.text(parent, text, "HUD/11", color, x=x, y=y, name=name or text)


def vstack(d, parent, name, x, y, w, gap=8, pad=None, fill=None, **kw):
    kw.setdefault("sizingV", "hug")
    return d.frame(parent, name, x, y, w, 10, fill=fill, layout="vertical", gap=gap,
                   padding=pad or [0, 0, 0, 0], clip=False, **kw)


def hstack(d, parent, name, x=0, y=0, gap=8, h=None, **kw):
    kw.setdefault("crossAlign", "center")
    kw.setdefault("sizingH", "hug")
    kw.setdefault("sizingV", "hug" if h is None else "fixed")
    return d.frame(parent, name, x, y, 10, h or 10, fill=kw.pop("fill", None), layout="horizontal", gap=gap,
                   clip=False, **kw)


def menu(d, parent, x, y, items, w=240, name="Menu"):
    """A square menu: hairline border, dialog shadow. items: (label, icon_app|None, trail|None, tone) or '-'."""
    m = d.frame(parent, name, x, y, w, 10, fill="bg", stroke="line", layout="vertical", padding=[4, 0, 4, 0],
                sizingV="hug", clip=False)
    d.effect_style(m, "Shadow/Dialog")
    for it in items:
        if it == "-":
            sep = d.frame(m, "Separator", 0, 0, w, 9, fill=None, layout="vertical", padding=[4, 0, 4, 0],
                          sizingH="fill")
            d.rect(sep, 0, 0, w, 1, "line", name="rule")
            d.set(d.sub(sep, "rule"), sizingH="fill")
            continue
        label, icon, trail, tone = (list(it) + [None, None, None])[:4]
        props = {"Label": label, "Icon": bool(icon), "Show trail": bool(trail),
                 "State": "Hover" if tone == "hover" else "Default"}
        if trail:
            props["Trail"] = trail
        iid = d.inst("Menu item", m, props=props, name=f"Item {label}")
        d.set(iid, sizingH="fill")
        if icon:
            d.R("image.replace", {"ids": [d.sub(iid, "Icon image")], "path": ICON(icon)})
        if tone == "danger":
            d.set(d.sub(iid, "Label"), fill="bad")
        if trail == "Exp.":
            d.set(d.sub(iid, "Trail"), fill="warn")
    return m


def scrim(d, f, w=W, h=H):
    r = d.rect(f, 0, 0, w, h, "scrim", name="Scrim")
    return r


def dialog(d, parent, x, y, w, title, name="Dialog", hud_label=None):
    dl = d.frame(parent, name, x, y, w, 10, fill="bg", stroke="line", layout="vertical", sizingV="hug", clip=False)
    d.effect_style(dl, "Shadow/Dialog")
    head = vstack(d, dl, "Dialog head", 0, 0, w, gap=6, pad=[22, 24, 0, 24])
    d.set(head, sizingH="fill")
    if hud_label:
        hud(d, head, hud_label)
    d.text(head, title, "Display/Dialog title 22", "ink_strong", name="Title")
    body = vstack(d, dl, "Dialog body", 0, 0, w, gap=12, pad=[14, 24, 22, 24])
    d.set(body, sizingH="fill")
    return dl, body


def dialog_actions(d, dl, buttons, w):
    row = d.frame(dl, "Dialog actions", 0, 0, w, 66, fill=None, layout="horizontal", gap=8, padding=[16, 24, 16, 24],
                  mainAlign="max", crossAlign="center", sizingH="fill", sizingV="hug")
    side_stroke(d, row, top=1)
    out = []
    for kind, label in buttons:
        out.append(d.inst("Button", row, props={"Kind": kind, "Label": label}, name=label))
    return row, out


def sheet(d, parent, x, y, w, h, title, hud_label, app=None):
    """The launcher's sheet: accent-tinted header (9 %) under a 2 px accent rule."""
    s = d.frame(parent, "Sheet", x, y, w, h, fill="bg", stroke="line", layout="vertical", clip=True)
    d.effect_style(s, "Shadow/Dialog")
    hd = d.frame(s, "Sheet header", 0, 0, w, 10, fill="accent_tint", layout="horizontal", gap=16,
                 padding=[24, 24, 20, 24], crossAlign="center", sizingH="fill", sizingV="hug", clip=False)
    side_stroke(d, hd, bottom=1)
    rule = d.rect(hd, 0, 0, w, 2, "accent", name="Accent rule")
    d.absolute(rule, 0, 0, w, 2)
    if app:
        ic = d.inst("App icon", hd, name="Sheet icon")
        d.R("image.replace", {"ids": [d.sub(ic, "Icon image")], "path": ICON(app)})
        d.set(ic, width=44, height=44)
    col = vstack(d, hd, "Sheet titles", 0, 0, 300, gap=6)
    d.set(col, sizingH="fill")
    hud(d, col, hud_label, color="accent_ink", name="Sheet kicker")
    d.text(col, title, "Display/Sheet title 26", "ink_strong", name="Sheet title")
    close = d.frame(hd, "Close", 0, 0, 34, 34, fill=None, stroke="line_strong", layout="horizontal",
                    mainAlign="center", crossAlign="center")
    d.inst("Icon/Close", close, name="Close glyph")
    return s


def duplicate_into(d, src, name, x, y):
    """A copy of a finished screen (as the backdrop of a menu or dialog)."""
    r = d.R("edit.duplicate", {"ids": [src]})
    cid = r["ids"][0]
    d.set(cid, name=name, x=x, y=y)
    return cid


# --- 1. launcher ------------------------------------------------------------------------------------

def card_grid(d, f, x0, y0, ids, cols=4, card_w=CARD_W, hover=None):
    out = {}
    for i, app in enumerate(ids):
        r, c = divmod(i, cols)
        var = "App card/Hover" if app == hover else "App card/Default"
        iid = d.inst(var, f, x0 + c * (card_w - 1), y0 + r * (CARD_H - 1), name=f"Card {APP[app]['name']}")
        if card_w != CARD_W:
            d.set(iid, width=card_w)
        fill_card(d, iid, app, hover=app == hover)
        out[app] = iid
    return out


def launcher_grid(d, name, x, y, light=False, full=False, menu_open=True):
    pinned, rest = ORDER[:PINNED], ORDER[PINNED:]
    rows = (len(rest) + 3) // 4
    h = (155 + CARD_H + 28 + 25 + rows * (CARD_H - 1) + 1 + 24 + 34) if full else H
    f = screen(d, name, x, y, h=h, light=light)
    header(d, f, "Apps")
    d.inst("Toolbar/Launcher", f, 24, 48 + 24, name="Toolbar")
    yy = 130
    hud(d, f, "Pinned", 24, yy, name="Pinned label")
    c1 = hud(d, f, f"{PINNED} apps", 0, yy, name="Pinned count")
    d.set(c1, x=W - 24 - d.R("node.get", {"id": c1, "fields": ["width"]})["width"])
    cards = card_grid(d, f, 24, yy + 25, pinned, hover=None if full else "wordcraft")
    yy += 25 + CARD_H + 28
    hud(d, f, "All apps", 24, yy, name="All label")
    c2 = hud(d, f, f"{len(rest)} apps", 0, yy, name="All count")
    d.set(c2, x=W - 24 - d.R("node.get", {"id": c2, "fields": ["width"]})["width"])
    cards.update(card_grid(d, f, 24, yy + 25, rest))
    if not full and menu_open:
        # The hovered card's "…" menu is open.
        more = d.sub(cards["wordcraft"], "More")
        d.set(more, opacity=1)
        bx = 24 + CARD_W - 20 - 34
        by = 155 + CARD_H - 20 - 34
        hl = d.inst("Button/More hover", f, bx, by, name="More (open)")
        menu(d, f, bx + 34 - 220, by + 38, [("Unpin", None), ("File types…", None), "-",
                                             ("About WordCraft 0.18.1", None)], w=220, name="Card menu")
        del hl
    footer(d, f, h)
    return f, cards


def toggle_table(d, toolbar):
    g = d.sub(toolbar, "Grid")
    t = d.sub(toolbar, "Table")
    d.set(g, fill=None)
    d.set(t, fill="invert_bg")
    for k in d.R("node.find", {"root": g, "type": "RECTANGLE"})["ids"]:
        d.set(k, fill="muted")
    for k in d.R("node.find", {"root": t, "type": "RECTANGLE"})["ids"]:
        d.set(k, fill="invert_fg")


def launcher_table(d, name, x, y):
    f = screen(d, name, x, y)
    header(d, f, "Apps")
    tb = d.inst("Toolbar/Launcher", f, 24, 72, name="Toolbar")
    toggle_table(d, tb)
    d.set([d.sub(tb, "Sort label"), d.sub(tb, "Sort")], visible=False)
    tw = sum(w for _, w in TABLE_COLS)
    tbl = d.frame(f, "Table", 24, 130, tw + 2, H - 34 - 24 - 130, fill="bg", stroke="line", clip=True)
    hd = d.frame(tbl, "Table header", 1, 1, tw, 42, fill=None, layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1)
    for lab, w in TABLE_COLS:
        c = d.frame(hd, f"Head {lab or 'actions'}", 0, 0, w, 42, fill=None, layout="horizontal",
                    padding=[0, 16, 0, 16], gap=6, crossAlign="center", sizingH="fixed", sizingV="fill")
        if lab:
            hud(d, c, lab, color="ink" if lab == "App" else "muted", name=f"{lab} label")
        if lab == "App":
            d.inst("Icon/Sort up", c, name="Sort arrow")
    for i, app in enumerate(ORDER):
        iid = d.inst("Table row", tbl, 1, 43 + i * 62, name=f"Row {APP[app]['name']}")
        fill_row(d, iid, app)
    # Hover on the second row: sunken fill.
    d.set(d.find(tbl, "Row GridCraft")[0], fill="sunken")
    footer(d, f)
    return f


def launcher_mobile(d, name, x, y):
    w, h = 390, 844
    f = screen(d, name, x, y, w, h)
    hd = d.frame(f, "Header", 0, 0, w, 48, fill="bg", layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1)
    mb = d.frame(hd, "Nav drawer", 0, 0, 48, 48, fill=None, layout="horizontal", mainAlign="center",
                 crossAlign="center")
    side_stroke(d, mb, right=1)
    d.inst("Icon/Menu", mb, name="Menu icon")
    br = d.frame(hd, "Brand", 0, 0, 10, 48, fill=None, layout="horizontal", padding=[0, 16, 0, 16],
                 crossAlign="center", sizingH="hug", sizingV="fill")
    d.inst("Brand/CraftCloud wordmark", br, name="Wordmark")
    d.frame(hd, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    us = d.frame(hd, "User", 0, 0, 48, 48, fill=None, layout="horizontal", mainAlign="center", crossAlign="center")
    side_stroke(d, us, left=1)
    av = d.frame(us, "Avatar", 0, 0, 28, 28, fill="sunken", stroke="line_strong", layout="horizontal",
                 mainAlign="center", crossAlign="center")
    d.text(av, USER[0], "Badge/10", "ink", name="Initials")
    # Section line: current section + view toggle.
    sec = d.frame(f, "Section", 16, 64, w - 32, 34, fill=None, layout="horizontal", gap=8, crossAlign="center")
    d.text(sec, "Apps", "Display/Heading 24", "ink_strong", name="Section title")
    d.frame(sec, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    s = d.inst("Search field", sec, props={"Placeholder": "Search"}, name="Search")
    d.set(s, width=150)
    filt = d.frame(f, "Filters", 16, 110, w - 16, 34, fill=None, clip=True)
    row = d.frame(filt, "Filter cells", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                  sizingV="fixed", clip=False)
    for i, (cid, lab, n) in enumerate(CATS):
        if i:
            d.rect(row, 0, 0, 1, 34, "line", name="sep")
        d.inst("Cell", row, props={"Label": lab, "Count": str(n), "State": "On" if cid == "all" else "Off"},
               name=f"Filter {lab}")
    hud(d, f, "Pinned", 16, 162, name="Pinned label")
    cw = w - 32
    yy = 186
    for i, app in enumerate(ORDER[:3]):
        iid = d.inst("App card/Default", f, 16, yy + i * (CARD_H - 1), name=f"Card {APP[app]['name']}")
        d.set(iid, width=cw)
        fill_card(d, iid, app)
    return f


# --- 3. files -----------------------------------------------------------------------------------------

def files_toolbar(d, f, wlist):
    tb = d.frame(f, "Files toolbar", 24, 72, wlist, 34, fill=None, layout="horizontal", gap=8, crossAlign="center",
                 clip=False)
    bc = d.frame(tb, "Breadcrumbs", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed")
    for i, part in enumerate(("Files", "Documents", "Reports")):
        if i:
            sep = d.frame(bc, "sep", 0, 0, 18, 34, fill=None, layout="horizontal", mainAlign="center",
                          crossAlign="center")
            side_stroke(d, sep, left=1, right=1)
            d.text(sep, "›", "Body/13", "muted", name="›")
        d.inst("Cell", bc, props={"Label": part, "Show count": False, "State": "On" if i == 2 else "Off"},
               name=f"Crumb {part}")
    d.frame(tb, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    d.inst("Search field", tb, props={"Placeholder": "Search in Reports"}, name="Search folder")
    d.inst("Button", tb, props={"Kind": "Primary", "Label": "Upload"}, name="Upload")
    d.inst("Button", tb, props={"Kind": "Secondary", "Label": "New folder"}, name="New folder")
    d.inst("Button/Menu Secondary", tb, props={"Label": "New"}, name="New menu")
    tg = d.frame(tb, "View toggle", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed")
    for lab, on in (("Grid", False), ("List", True)):
        c = d.frame(tg, lab, 0, 0, 34, 34, fill="invert_bg" if on else None, layout="horizontal",
                    mainAlign="center", crossAlign="center")
        ic = d.inst(f"Icon/{lab}", c, name=f"{lab} icon")
        if on:
            for k in d.R("node.find", {"root": ic, "type": "RECTANGLE"})["ids"]:
                d.set(k, fill="invert_fg")
    return tb


def file_row(d, parent, x, y, fl, selected=False, hover=False):
    name, app, mod, size, shared = fl
    state = "Selected" if selected else ("Hover" if hover else "Default")
    iid = d.inst("File row", parent, x, y, name=f"Row {name}")
    props = {"State": state, "Name": name, "Modified": mod, "Size": size, "Shared": shared}
    if app == "folder":
        props.update({"App": "—", "App icon": False})
    elif app is None:
        props.update({"App": "↓ Download", "App icon": False})
    else:
        props.update({"App": APP[app]["name"]})
    d.R("instance.setProperties", {"ids": [iid], "properties": props})
    if app == "folder":
        d.set(d.sub(iid, "File icon"), visible=False)
        d.set(d.sub(iid, "Folder icon"), visible=True)
        d.set(d.sub(iid, "Opens app"), fill="muted")
    elif app is None:
        d.set(d.sub(d.sub(iid, "File icon"), "page"), fill="muted")
        d.set(d.sub(iid, "Opens app"), fill="muted")
    else:
        d.set(d.sub(d.sub(iid, "File icon"), "page"), fill=f"app:{app}/color")
        d.R("image.replace", {"ids": [d.sub(iid, "Icon image")], "path": ICON(app)})
    return iid


def doc_preview(d, parent, w, h):
    pv = d.frame(parent, "Preview", 0, 0, w, h, fill="sunken", stroke="line", clip=True)
    pw, ph = 118, 152
    pg = d.frame(pv, "Page", (w - pw) / 2, (h - ph) / 2 + 6, pw, ph, fill="#ffffff")
    d.effect_style(pg, "Shadow/App icon")
    d.rect(pg, 12, 14, 70, 7, "#1d2240", name="Title line")
    d.rect(pg, 12, 26, 40, 4, "#3f5fe0", name="Subtitle line")
    for i in range(9):
        d.rect(pg, 12, 40 + i * 8, pw - 24 - (18 if i % 4 == 3 else 0), 3, "#c9cbd3", name=f"line {i}")
    d.rect(pg, 12, 116, 44, 24, "#dfe4fb", name="Chart")
    d.rect(pg, 60, 116, 46, 24, "#e8eaf0", name="Table")
    return pv


def details_panel(d, f, x, light=False):
    p = d.frame(f, "Details", x, 48, DETAILS_W, H - 48 - 34, fill="bg", layout="vertical", padding=[24, 24, 24, 24],
                gap=18, clip=True)
    side_stroke(d, p, left=1)
    doc_preview(d, p, DETAILS_W - 48, 150)
    nm = vstack(d, p, "Name block", 0, 0, DETAILS_W - 48, gap=4)
    d.text(nm, "Q3 report.docx", "Display/Dialog title 22", "ink_strong", name="Detail name")
    d.text(nm, "Word document · opens in WordCraft", "Small/12.5", "muted", name="Detail kind")
    ac = hstack(d, p, "Detail actions", gap=8)
    d.set(ac, sizingH="fill")
    o = d.inst("Button", ac, props={"Kind": "Primary", "Label": "Open in WordCraft"}, name="Open in app")
    d.set(o, sizingH="fill")
    d.inst("Button/Menu Secondary", ac, props={"Label": "Open with"}, name="Open with")
    facts = vstack(d, p, "Facts", 0, 0, DETAILS_W - 48, gap=6)
    for k, v in (("Size", "248 KB"), ("Modified", "Today 11:42 by John Doe"), ("Owner", "John Doe"),
                 ("Path", "/Documents/Reports")):
        r = hstack(d, facts, f"Fact {k}", gap=0)
        lab = hud(d, r, k, name=f"{k} key")
        d.set(lab, sizingH="fixed", width=96)
        d.text(r, v, "Mono/12" if k in ("Size", "Path") else "Body/13", "ink", name=f"{k} value")
    vs = vstack(d, p, "Versions", 0, 0, DETAILS_W - 48, gap=0)
    vh = hstack(d, vs, "Versions head", gap=8, padding=[0, 0, 8, 0])
    d.set(vh, sizingH="fill")
    hud(d, vh, "Versions", color="ink", name="Versions title")
    d.frame(vh, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    hud(d, vh, "Last 5 kept", name="Versions note")
    for i, (cur, when, size, by) in enumerate(VERSIONS):
        r = d.frame(vs, f"Version {i}", 0, 0, DETAILS_W - 48, 30, fill=None, layout="horizontal", gap=8,
                    crossAlign="center", sizingH="fill", sizingV="fixed")
        side_stroke(d, r, top=1)
        d.text(r, when, "Body/13 Medium" if cur else "Body/13", "ink", name="When")
        d.text(r, size, "Mono/12", "muted", name="Size")
        d.frame(r, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
        if cur:
            d.inst("Status badge", r, props={"Tone": "Good", "Label": "Current"}, name="Current")
        else:
            d.text(r, "Restore", "Button/11", "accent_ink", name="Restore")
    sl = vstack(d, p, "Share link", 0, 0, DETAILS_W - 48, gap=8)
    hud(d, sl, "Share link", color="ink", name="Share title")
    r = hstack(d, sl, "Link row", gap=8)
    d.set(r, sizingH="fill")
    li = d.inst("Input", r, props={"Value": "cloud.example.org/s/7Kq2xW"}, name="Link")
    d.set(li, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Copy"}, name="Copy")
    d.text(sl, "Anyone with the link can view · expires 13 Dec 2026", "Small/12", "muted", name="Link note")
    return p


def files_view(d, name, x, y, light=False, new_menu=False):
    f = screen(d, name, x, y, light=light)
    header(d, f, "Files")
    wlist = W - DETAILS_W - 48
    files_toolbar(d, f, wlist)
    lst = d.frame(f, "File list", 24, 130, wlist, 40 + 48 * len(FILES), fill=None, clip=False)
    hd = d.frame(lst, "List header", 0, 0, wlist, 40, fill=None, layout="horizontal", crossAlign="center")
    side_stroke(d, hd, bottom=1, top=1)
    for lab, w in FILE_COLS:
        c = d.frame(hd, f"Head {lab or 'more'}", 0, 0, w or 100, 40, fill=None, layout="horizontal",
                    padding=[0, 12, 0, 16 if lab == "Name" else 12], gap=6, crossAlign="center",
                    sizingH="fill" if w is None else "fixed", sizingV="fill")
        if lab:
            hud(d, c, lab, color="ink" if lab == "Name" else "muted", name=f"{lab} label")
        if lab == "Name":
            d.inst("Icon/Sort up", c, name="Sort arrow")
    rows = {}
    for i, fl in enumerate(FILES):
        rows[fl[0]] = file_row(d, lst, 0, 40 + i * 48, fl, selected=fl[0] == "Q3 report.docx",
                               hover=(fl[0] == "Logo.svg" and not new_menu))
    hud(d, f, f"{len(FILES)} items · 625 MB", 24, 130 + 40 + 48 * len(FILES) + 14, name="List summary")
    details_panel(d, f, W - DETAILS_W, light)
    if new_menu:
        nb = d.sub(f, "New menu")
        bx = 24 + wlist - 34 - 8 - 98
        items = [("Document in WordCraft", "wordcraft", None, "hover"), ("Spreadsheet in GridCraft", "gridcraft"),
                 ("Presentation in DeckCraft", "deckcraft"), ("Design in FigCraft", "figcraft"),
                 ("Drawing in CADCraft", "cadcraft"), ("Video project in FilmCraft", "filmcraft"), "-",
                 ("Image in PhotoCraft", "photocraft", "Exp."), ("Illustration in VectorCraft", "vectorcraft", "Exp."),
                 ("Layout in DesignCraft", "designcraft", "Exp."), ("Audio in SoundCraft", "soundcraft", "Exp."),
                 ("Motion in EffectCraft", "effectcraft", "Exp."), ("Model in TextureCraft", "texturecraft", "Exp."),
                 ("Material in MaterialCraft", "materialcraft", "Exp.")]
        nbb = d.R("node.get", {"id": nb, "fields": ["absoluteBounds"]})["absoluteBounds"]
        fb = d.R("node.get", {"id": f, "fields": ["absoluteBounds"]})["absoluteBounds"]
        mw = 280
        menu(d, f, nbb["x"] - fb["x"] + nbb["width"] - mw, nbb["y"] - fb["y"] + 38, items, w=mw,
             name="New menu (open)")
        d.R("node.set", {"ids": [nb], "props": {"fills": [{"type": "solid", "color": "#ffffff"}]}})
        d.R("variable.bind", {"ids": [nb], "field": "fills/0/color", "variable": "invert_bg"})
        for k in d.R("node.find", {"root": nb, "type": "TEXT"})["ids"]:
            d.set(k, fill="invert_fg")
    footer(d, f, left="12.4 GB of 100 GB used · versions kept: 5")
    return f, rows


# --- 5/6. editor ---------------------------------------------------------------------------------------

def editor(d, name, x, y):
    f = screen(d, name, x, y)
    bar = d.inst("Editor bar", f, 0, 0, name="Editor bar")
    d.R("instance.setProperties", {"ids": [bar], "properties": {"State": "Saved"}})
    app = d.frame(f, "WordCraft (app iframe)", 0, 40, W, H - 40, fill="sunken", clip=True)
    # The screenshot is a whole desktop window; crop its own title bar and border so only the app shows.
    iw = W + 26
    d.image(app, os.path.join(ASSETS, "shots", "wordcraft-hero.jpg"), -13, -40, iw, iw * 750 / 1200,
            name="WordCraft screenshot", mode="fill")
    return f


def editor_states(d, name, x, y):
    h = 1140
    f = screen(d, name, x, y, h=h)
    yy = 32
    d.text(f, "Editor bar states", "Display/Heading 24", "ink_strong", x=24, y=yy, name="Title")
    yy += 52
    notes = {"Opening": "Loading the file into WordCraft; the bar's progress line is in the app's colour.",
             "Unsaved": "WordCraft reported unsaved changes (dirty). Leaving asks first.",
             "Saving": "The save is being written to the cloud; the old bytes went to .craftcloud/versions first.",
             "Saved": "Saved over the file. The time is when the cloud confirmed it.",
             "Failed": "The upload failed. Nothing is lost: retry, or download a copy.",
             "Read-only": "No write permission (a share or a locked file)."}
    for st in ("Opening", "Unsaved", "Saving", "Saved", "Failed", "Read-only"):
        r = hstack(d, f, f"Label {st}", 24, yy, gap=12)
        hud(d, r, st, color="ink", name="State")
        d.text(r, notes[st], "Small/12", "muted", name="Note")
        b = d.inst("Editor bar", f, 0, yy + 24, name=f"Bar {st}")
        d.R("instance.setProperties", {"ids": [b], "properties": {"State": st}})
        yy += 24 + 40 + 28
    yy += 12
    d.text(f, "Toasts", "Display/Heading 24", "ink_strong", x=24, y=yy, name="Toasts title")
    yy += 48
    toast(d, f, 24, yy, "good", "Saved as a new file", "Q3 report.doc → Q3 report.docx. The .doc file is unchanged.",
          "Open folder")
    toast(d, f, 24 + 640, yy, "warn", "Someone changed this file while you edited",
          "Your version was saved as 'Q3 report (conflict 14.32).docx'.", "Compare")
    yy += 110
    d.text(f, "Leave guard", "Display/Heading 24", "ink_strong", x=24, y=yy, name="Dialog title")
    yy += 48
    stage = d.frame(f, "Dialog stage", 24, yy, W - 48, 300, fill="sunken", stroke="line", clip=True)
    d.image(stage, os.path.join(ASSETS, "shots", "wordcraft-hero.jpg"), 0, -120, W - 48, (W - 48) * 750 / 1200,
            name="Editor behind")
    d.rect(stage, 0, 0, W - 48, 300, "scrim", name="Scrim")
    dl, body = dialog(d, stage, (W - 48 - 480) / 2, 40, 480, "Leave with unsaved changes?")
    d.text(body, "Q3 report.docx has changes WordCraft hasn't saved to the cloud yet. If you leave now, they are lost.",
           "Body/14", "ink", name="Message", w=432)
    dialog_actions(d, dl, [("Danger", "Discard"), ("Primary", "Stay")], 480)
    return f


def toast(d, parent, x, y, tone, title, msg, action):
    t = d.frame(parent, f"Toast {title}", x, y, 600, 10, fill="bg", stroke="line", layout="horizontal", gap=14,
                padding=[14, 14, 14, 18], crossAlign="center", sizingV="hug", clip=False)
    d.effect_style(t, "Shadow/Dialog")
    bar = d.rect(t, 0, 0, 2, 10, tone, name="Tone rule")
    d.absolute(bar, 0, 0)
    col = vstack(d, t, "Toast text", 0, 0, 400, gap=4)
    d.set(col, sizingH="fill")
    d.text(col, title, "Body/14 Medium", "ink_strong", name="Toast title")
    d.text(col, msg, "Small/12.5", "muted", name="Toast message", w=400)
    d.set(d.sub(col, "Toast message"), sizingH="fill")
    b = d.inst("Button", t, props={"Kind": "Secondary", "Label": action}, name="Toast action")
    d.set(b, height=28)
    x_ = d.inst("Icon/Close", t, name="Dismiss")
    d.set(d.sub(x_, "glyph"), stroke="muted")
    d.set(bar, x=0, y=0, height=d.R("node.get", {"id": t, "fields": ["height"]})["height"])
    return t


# --- 7/8/10. dialogs and sheets --------------------------------------------------------------------------

def new_document(d, name, x, y, backdrop):
    f = duplicate_into(d, backdrop, name, x, y)
    for k in ("Card menu", "More (open)"):
        for i in d.find(f, k):
            d.R("node.delete", {"ids": [i]}) if False else d.set(i, visible=False)
    scrim(d, f)
    sw, sh = 640, 520
    s = sheet(d, f, (W - sw) / 2, 110, sw, sh, "New document in WordCraft", "New · WordCraft 0.18.1", app="wordcraft")
    body = vstack(d, s, "Sheet body", 0, 0, sw, gap=20, pad=[24, 24, 24, 24])
    d.set(body, sizingH="fill", sizingV="fill")
    fld = vstack(d, body, "Field name", 0, 0, sw - 48, gap=8)
    hud(d, fld, "File name", color="ink", name="Name label")
    r = hstack(d, fld, "Name row", gap=8)
    d.set(r, sizingH="fill")
    inp = d.inst("Input", r, props={"State": "Focus", "Value": "Q4 plan"}, name="File name input")
    d.set(inp, sizingH="fill")
    d.text(r, ".docx", "Mono/13", "muted", name="Extension")
    fld = vstack(d, body, "Field folder", 0, 0, sw - 48, gap=8)
    hud(d, fld, "Folder", color="ink", name="Folder label")
    r = hstack(d, fld, "Folder row", gap=8)
    d.set(r, sizingH="fill")
    sel = d.inst("Select", r, props={"Value": "Documents"}, name="Folder picker")
    d.set(sel, sizingH="fill")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Browse…"}, name="Browse")
    fld = vstack(d, body, "Field format", 0, 0, sw - 48, gap=8)
    hud(d, fld, "Format", color="ink", name="Format label")
    cells = d.frame(fld, "Format cells", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                    sizingV="fixed")
    for i, (lab, on) in enumerate((("Word .docx", True), ("OpenDocument .odt", False), ("Markdown .md", False),
                                   ("Plain text .txt", False))):
        if i:
            d.rect(cells, 0, 0, 1, 34, "line", name="sep")
        d.inst("Cell", cells, props={"Label": lab, "Show count": False, "State": "On" if on else "Off"},
               name=f"Format {lab}")
    d.text(body, "Creates Documents/Q4 plan.docx in your cloud and opens it in WordCraft. Every save goes back to "
                 "that file; the last 5 versions are kept.", "Body/14", "muted", name="Explain", w=sw - 48)
    d.frame(body, "Spacer", 0, 0, 10, 10, fill=None, sizingV="fill")
    row = hstack(d, s, "Sheet actions", gap=8, padding=[16, 24, 16, 24], mainAlign="max")
    d.set(row, sizingH="fill")
    side_stroke(d, row, top=1)
    d.inst("Button", row, props={"Kind": "Secondary", "Label": "Cancel"}, name="Cancel")
    d.inst("Button", row, props={"Kind": "Primary", "Label": "Create"}, name="Create")
    return f


def versions_dialog(d, name, x, y, backdrop):
    f = duplicate_into(d, backdrop, name, x, y)
    scrim(d, f)
    w = 760
    dl, body = dialog(d, f, (W - w) / 2, 150, w, "Versions of Q3 report.docx", hud_label="Documents › Reports")
    d.text(body, "CraftCloud keeps the last 5 versions. Restoring puts the current file into the list first, so "
                 "nothing is lost.", "Body/14", "muted", name="Note", w=w - 48)
    tbl = d.frame(body, "Versions table", 0, 0, w - 48, 10, stroke="line", layout="vertical", sizingV="hug",
                  sizingH="fill")
    cols = (("Saved", 220), ("Size", 100), ("Saved by", 160), ("", 230))
    hd = hstack(d, tbl, "Head", gap=0, h=38)
    d.set(hd, sizingH="fill")
    side_stroke(d, hd, bottom=1)
    for lab, cw in cols:
        c = d.frame(hd, f"H {lab}", 0, 0, cw, 38, fill=None, layout="horizontal", padding=[0, 14, 0, 14],
                    crossAlign="center", sizingV="fill")
        if lab:
            hud(d, c, lab, name=f"{lab} label")
    for i, (cur, when, size, by) in enumerate(VERSIONS):
        r = hstack(d, tbl, f"Version {i}", gap=0, h=52)
        d.set(r, sizingH="fill")
        if i < len(VERSIONS) - 1:
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
        if by == "WordCraft":
            d.inst("App icon/small", c, name="By icon")
        d.text(c, by, "Body/13", "ink" if by == "WordCraft" else "muted", name="By name")
        c = d.frame(r, "Actions", 0, 0, cols[3][1], 52, fill=None, layout="horizontal", padding=[0, 14, 0, 14], gap=8,
                    crossAlign="center", mainAlign="max", sizingV="fill", sizingH="fill")
        if not cur:
            st = "Focus" if i == 1 else "Default"
            d.inst("Button", c, props={"Kind": "Secondary", "Label": "Restore", "State": st}, name="Restore")
        d.inst("Button", c, props={"Kind": "Secondary", "Label": "Download"}, name="Download")
    dialog_actions(d, dl, [("Primary", "Close")], w)
    return f


def settings(d, name, x, y, backdrop):
    f = duplicate_into(d, backdrop, name, x, y)
    for k in ("Card menu", "More (open)"):
        for i in d.find(f, k):
            d.set(i, visible=False)
    scrim(d, f)
    sw, sh = 1120, 780
    s = sheet(d, f, (W - sw) / 2, 60, sw, sh, "Settings", "CraftCloud " + VERSION)
    body = d.frame(s, "Sheet body", 0, 0, sw, 10, fill=None, layout="horizontal", gap=24, padding=[24, 24, 24, 24],
                   sizingH="fill", sizingV="fill", crossAlign="min", clip=False)
    colw = (sw - 48 - 24) / 2
    left = vstack(d, body, "Left column", 0, 0, colw, gap=24)
    right = vstack(d, body, "Right column", 0, 0, colw, gap=24)

    def panel(parent, title):
        p = vstack(d, parent, f"Panel {title}", 0, 0, colw, gap=12, pad=[16, 18, 18, 18])
        d.set(p, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
        d.R("variable.bind", {"ids": [p], "field": "strokes/0/color", "variable": "line"})
        hud(d, p, title, color="ink", name="Panel title")
        return p

    p = panel(left, "Appearance")
    cells = d.frame(p, "Theme cells", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                    sizingV="fixed")
    for i, (lab, on) in enumerate((("System", False), ("Dark", True), ("Light", False))):
        if i:
            d.rect(cells, 0, 0, 1, 34, "line", name="sep")
        d.inst("Cell", cells, props={"Label": lab, "Show count": False, "State": "On" if on else "Off"},
               name=f"Theme {lab}")
    p = panel(left, "Default apps")
    d.text(p, "Which Craft app opens a file when you click it. Other apps stay under Open with.", "Small/12.5", "muted",
           name="Default apps note", w=colw - 36)
    groups = [(".docx .odt .rtf .md .txt", "wordcraft"), (".xlsx .ods .csv", "gridcraft"), (".pptx .potx", "deckcraft"),
              (".pdf", "pdfcraft"), (".psd .png .jpg .webp", "photocraft"), (".cr3 .nef .dng (raw)", "lightcraft"),
              (".svg .ai .eps", "vectorcraft"), (".dwg .dxf", "cadcraft"), (".mp4 .mov .webm", "filmcraft")]
    tb = vstack(d, p, "Default apps table", 0, 0, colw - 36, gap=0)
    d.set(tb, sizingH="fill")
    for ext, app in groups:
        r = hstack(d, tb, f"Assoc {ext}", gap=8, h=42)
        d.set(r, sizingH="fill")
        side_stroke(d, r, top=1)
        e = d.text(r, ext.upper(), "HUD/11", "ink", name="Extensions")
        d.set(e, sizingH="fill")
        d.text(r, "→", "Body/13", "muted", name="arrow")
        sel = d.frame(r, "App picker", 0, 0, 200, 32, fill="bg", stroke="line_strong", layout="horizontal",
                      padding=[0, 10, 0, 8], gap=8, crossAlign="center", sizingV="fixed", sizingH="fixed")
        ic = d.inst("App icon/small", sel, name="Picker icon")
        d.set(ic, width=16, height=16)
        d.R("image.replace", {"ids": [d.sub(ic, "Icon image")], "path": ICON(app)})
        t = d.text(sel, APP[app]["name"], "Body/13", "ink", name="Picker value")
        d.set(t, sizingH="fill")
        d.inst("Icon/Chevron down", sel, name="Picker chevron")
    p = panel(right, "Storage")
    hstack_ = hstack(d, p, "Storage line", gap=8)
    d.text(hstack_, STORAGE[0] + " used", "Body/14 Medium", "ink", name="Storage used")
    bar = d.frame(p, "Meter", 0, 0, colw - 36, 4, fill="line")
    d.set(bar, sizingH="fill")
    d.rect(bar, 0, 0, round((colw - 36) * STORAGE[1]), 4, "accent", name="Used")
    for k, v in (("Versions kept", "5 per file (.craftcloud/versions)"), ("Trash", "1.2 GB · emptied after 30 days")):
        r = hstack(d, p, k, gap=0)
        lab = hud(d, r, k, name=f"{k} key")
        d.set(lab, sizingH="fixed", width=150)
        d.text(r, v, "Body/13", "ink", name=f"{k} value")
    p = panel(right, "Account")
    r = hstack(d, p, "Account line", gap=10)
    av = d.frame(r, "Avatar", 0, 0, 28, 28, fill="sunken", stroke="line_strong", layout="horizontal",
                 mainAlign="center", crossAlign="center")
    d.text(av, USER[0], "Badge/10", "ink", name="Initials")
    d.text(r, f"Signed in as {USER[1]}", "Body/14", "ink", name="Signed in")
    r = hstack(d, p, "Account actions", gap=8)
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Admin settings ↗"}, name="Admin settings")
    d.inst("Button", r, props={"Kind": "Secondary", "Label": "Sign out"}, name="Sign out")
    d.text(p, "Admin settings open OxiCloud's own pages (users, quotas, sign-in options).", "Small/12.5", "muted",
           name="Admin note", w=colw - 36)
    p = panel(right, "About")
    for k, v in (("CraftCloud", VERSION), ("OxiCloud", OXICLOUD + " (backend, unmodified)")):
        r = hstack(d, p, k, gap=0)
        lab = hud(d, r, k, name=f"{k} key")
        d.set(lab, sizingH="fixed", width=150)
        d.text(r, v, "Mono/13", "ink", name=f"{k} value")
    grid = d.frame(p, "App versions", 0, 0, colw - 36, 10, fill=None, layout="grid", gridColumns=3, columnGap=12,
                   rowGap=4, sizingV="hug", clip=False)
    d.set(grid, sizingH="fill")
    for app in ORDER:
        r = hstack(d, grid, f"V {app}", gap=6)
        d.text(r, APP[app]["name"], "Small/12", "ink", name="App")
        d.text(r, APP[app]["version"], "Mono/12", "muted", name="Ver")
    d.text(p, "CraftCloud is open source (MIT OR Apache-2.0). App icons from craft-launcher (MIT OR Apache-2.0). "
              "Fonts: Inter, Archivo, Geist Mono (SIL OFL 1.1). Look based on ArtCraft Launcher by the ArtCraft team.",
           "Small/12", "muted", name="Licences", w=colw - 36)
    return f


def login(d, name, x, y):
    f = screen(d, name, x, y)
    cw = 420
    card = vstack(d, f, "Sign-in card", (W - cw) / 2, 170, cw, gap=0, fill="bg")
    d.set(card, strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [card], "field": "strokes/0/color", "variable": "line"})
    top = vstack(d, card, "Brand block", 0, 0, cw, gap=10, pad=[36, 32, 28, 32])
    d.set(top, sizingH="fill")
    side_stroke(d, top, bottom=1)
    wm = d.inst("Brand/CraftCloud wordmark", top, name="Wordmark")
    d.R("transform.scale", {"ids": [wm], "factor": 1.6})
    hud(d, top, "Craft app suite", name="Suite")
    form = vstack(d, card, "Form", 0, 0, cw, gap=16, pad=[28, 32, 32, 32])
    d.set(form, sizingH="fill")
    for lab, val, st in (("Username", "john", "Focus"), ("Password", "••••••••••••", "Default")):
        fl = vstack(d, form, f"Field {lab}", 0, 0, cw - 64, gap=8)
        d.set(fl, sizingH="fill")
        hud(d, fl, lab, color="ink", name=f"{lab} label")
        i = d.inst("Input", fl, props={"State": st, "Value": val}, name=f"{lab} input")
        d.set(i, sizingH="fill")
    b = d.inst("Button", form, props={"Kind": "Primary", "Label": "Sign in"}, name="Sign in")
    d.set(b, sizingH="fill")
    r = hstack(d, form, "Other options", gap=6)
    d.text(r, "Other sign-in options", "Body/14", "accent_ink", name="Other link", textDecoration="underline")
    d.text(r, "↗", "Body/14", "accent_ink", name="Other arrow")
    d.text(form, "Single sign-on and email links continue on the server's own sign-in page.", "Small/12.5", "muted",
           name="Other note", w=cw - 64)
    ft = d.frame(f, "Login footer", 0, H - 48, W, 48, fill=None, layout="horizontal", mainAlign="center",
                 crossAlign="center")
    d.text(ft, f"CraftCloud {VERSION} · self-hosted · look based on ArtCraft Launcher by the ArtCraft team", "HUD/11",
           "muted", name="Footer line")
    return f


def build_all(d, only=None):
    frames = []

    def want(n):
        return only is None or n in only

    def add(n, fid):
        frames.append((n, fid))

    p1 = d.page("1 Launcher")
    f1, _ = launcher_grid(d, "launcher-grid-dark", 0, 0)
    add("launcher-grid-dark", f1)
    f, _ = launcher_grid(d, "launcher-grid-light", W + GAP, 0, light=True)
    add("launcher-grid-light", f)
    f = launcher_table(d, "launcher-table-dark", 2 * (W + GAP), 0)
    add("launcher-table-dark", f)
    f = launcher_mobile(d, "launcher-grid-mobile-dark", 3 * (W + GAP), 0)
    add("launcher-grid-mobile-dark", f)
    f, _ = launcher_grid(d, "launcher-grid-full-dark", 0, H + GAP, full=True)
    add("launcher-grid-full-dark", f)

    p2 = d.page("2 Files")
    fd, rows = files_view(d, "files-dark", 0, 0)
    add("files-dark", fd)
    f, _ = files_view(d, "files-light", W + GAP, 0, light=True, new_menu=True)
    add("files-light", f)
    fc = duplicate_into(d, fd, "files-context-menu-dark", 2 * (W + GAP), 0)
    context_menu(d, fc)
    add("files-context-menu-dark", fc)

    d.page("3 Editor")
    add("editor-wordcraft-dark", editor(d, "editor-wordcraft-dark", 0, 0))
    add("editor-states-dark", editor_states(d, "editor-states-dark", W + GAP, 0))

    d.page("4 Dialogs")
    # Backdrops are copied across pages: duplicate on the launcher page, then move.
    add("new-document-dialog-dark", copy_to_page(d, f1, p1, "new-document-dialog-dark", 0, 0, new_document))
    add("versions-dialog-dark", copy_to_page(d, fd, p2, "versions-dialog-dark", W + GAP, 0, versions_dialog))
    add("settings-dark", copy_to_page(d, f1, p1, "settings-dark", 2 * (W + GAP), 0, settings))

    d.page("5 Sign in")
    add("login-dark", login(d, "login-dark", 0, 0))
    return [fr for fr in frames if want(fr[0])]


def copy_to_page(d, src, src_page, name, x, y, builder):
    """Build a dialog frame over a copy of `src` (on its page), then move it to the current page."""
    cur = d.current_page
    d.R("page.select", {"id": src_page})
    fid = builder(d, name, 0, 5000, src)
    d.R("node.reparent", {"ids": [fid], "parent": cur})
    d.R("page.select", {"id": cur})
    d.set(fid, x=x, y=y)
    return fid


def context_menu(d, f):
    d.R("instance.setProperties", {"ids": [d.sub(f, "Row Logo.svg")], "properties": {"State": "Default"}})
    row = d.sub(f, "Row Contract.pdf")
    d.R("instance.setProperties", {"ids": [row], "properties": {"State": "Hover"}})
    d.set(d.sub(row, "More"), fill="ink_strong")
    ry = d.R("node.get", {"id": row, "fields": ["absoluteBounds"]})["absoluteBounds"]
    fb = d.R("node.get", {"id": f, "fields": ["absoluteBounds"]})["absoluteBounds"]
    mx = ry["x"] - fb["x"] + ry["width"] - 250
    my = ry["y"] - fb["y"] + 40
    m = menu(d, f, mx, my, [
        ("Open in PdfCraft", "pdfcraft"), ("Open with", None, "›", "hover"), "-",
        ("Download", None), ("Rename…", None, "F2"), ("Move to…", None), ("Copy to…", None),
        ("Share link…", None), ("Add to favourites", None), ("Versions…", None), "-",
        ("Delete", None, "Del", "danger")], w=240, name="Row menu")
    mb = d.R("node.get", {"id": m, "fields": ["absoluteBounds", "height"]})
    sub = menu(d, f, mx - 236, my + 4 + 32, [("PdfCraft", "pdfcraft", "Default"), ("VectorCraft", "vectorcraft", "Exp."),
                                             "-", ("Choose default app…", None)], w=230, name="Open with submenu")
    del mb, sub
