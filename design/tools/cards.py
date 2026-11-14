"""App card and table row components, and helpers that fill an instance for one app."""

from lib import APP, BLURB, RECENT
from components import ICON

CARD_W, CARD_H = 349, 314


def opens_label(a, n=3):
    if a["id"] == "figcraft":
        n = 2
    exts = " ".join(e.upper() for e in a["opens"][:n])
    return f"Opens {exts} …" if a["id"] in ("wordcraft", "gridcraft", "photocraft", "vectorcraft", "lightcraft",
                                             "texturecraft", "soundcraft", "filmcraft", "figcraft") else f"Opens {exts}"


def actions_for(a):
    if a["id"] == "lightcraft":
        return ("Open library", "Add photos…")
    return ("New", "Open file…")


def card_component(d, lib):
    y = lib.row(CARD_H)
    c = d.comp("State=Default", 0, y, CARD_W, CARD_H, fill="bg", stroke="line", layout="vertical", clip=True)
    rule = d.rect(c, 1, 1, CARD_W - 2, 2, "app:wordcraft/color", name="Hover rule")
    d.absolute(rule, 1, 1)
    d.set(rule, visible=False)
    strip = d.frame(c, "Strip", 0, 0, CARD_W, 40, fill=None, layout="horizontal", padding=[0, 20, 0, 20],
                    crossAlign="center", sizingH="fill", sizingV="fixed", strokes=[{"type": "solid", "color": "#fff"}],
                    strokeBottomWeight=1, strokeTopWeight=0, strokeLeftWeight=0, strokeRightWeight=0, strokeAlign="inside")
    d.R("variable.bind", {"ids": [strip], "field": "strokes/0/color", "variable": "line"})
    v = d.text(strip, "0.18.1", "HUD/11", "app:wordcraft/ink", name="Version")
    d.frame(strip, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    o = d.text(strip, "Opens .DOCX .ODT .MD …", "HUD/11", "muted", name="Opens")
    body = d.frame(c, "Body", 0, 0, CARD_W, 200, fill=None, layout="vertical", padding=[18, 20, 20, 20], gap=14,
                   sizingH="fill", sizingV="fill", clip=False)
    head = d.frame(body, "Head", 0, 0, 300, 56, fill=None, layout="horizontal", gap=14, crossAlign="center",
                   sizingH="fill", sizingV="hug", clip=False)
    wrap = d.frame(head, "Icon slot", 0, 0, 56, 56, fill=None, clip=False)
    d.inst("App icon", wrap, 0, 0, name="Icon")
    col = d.frame(head, "Names", 0, 0, 200, 50, fill=None, layout="vertical", gap=4, sizingH="fill", sizingV="hug",
                  clip=False)
    d.inst("App wordmark/22", col, name="Wordmark")
    tg = d.text(col, "Writing and document design", "Body/13.5 Tagline", "muted", name="Tagline")
    d.set(tg, sizingH="fill", textAutoResize="truncate", maxLines=1)
    info = d.frame(body, "Info", 0, 0, 300, 50, fill=None, layout="vertical", gap=8, sizingH="fill", sizingV="hug",
                   clip=False)
    exp = d.inst("Status badge", info, props={"Tone": "Warn", "Label": "Experimental"}, name="Experimental")
    bl = d.text(info, BLURB["wordcraft"], "Small/12.5", "muted", name="Blurb", w=300)
    d.set(bl, sizingH="fill")
    rc = d.text(info, RECENT["wordcraft"], "Small/12.5", "muted", name="Recent")
    d.frame(body, "Spacer", 0, 0, 10, 10, fill=None, sizingV="fill")
    act = d.frame(body, "Actions", 0, 0, 300, 34, fill=None, layout="horizontal", gap=8, sizingH="fill",
                  sizingV="hug", clip=False)
    p = d.inst("Button", act, props={"Kind": "Primary", "Label": "New"}, name="Primary")
    s = d.inst("Button", act, props={"Kind": "Secondary", "Label": "Open file…"}, name="Secondary")
    d.set([p, s], sizingH="fill")
    d.inst("Button/More", act, name="More")
    for lay, name in ((v, "Version"), (o, "Opens"), (tg, "Tagline"), (bl, "Blurb"), (rc, "Recent")):
        d.prop(c, lay, name)
    d.R("component.addProperty", {"id": c, "name": "Experimental", "type": "boolean", "layer": exp, "default": False})
    d.set(exp, visible=False)

    hv = d.R("component.addVariant", {"id": c, "property": "State", "value": "Hover"})["id"]
    d.set(d.sub(hv, "Hover rule"), visible=True)
    d.R("fill.add", {"ids": [hv], "color": "#3f5fe00d"})
    d.R("variable.bind", {"ids": [hv], "field": "fills/1/color", "variable": "wordcraft/wash"})
    d.set(d.sub(hv, "Icon"), y=-4)
    sets = d.R("node.get", {"id": c, "fields": ["name"]})
    parent = d.R("node.find", {"name": "State=Default", "type": "COMPONENT_SET", "limit": 5})
    # The set is the parent of both variants.
    s_id = None
    for i in d.R("node.find", {"type": "COMPONENT_SET", "limit": 50})["ids"]:
        kids = d.R("node.get", {"id": i, "fields": ["name"]}).get("children", [])
        if any(k["id"] == c for k in kids):
            s_id = i
    d.set(s_id, name="App card", x=0, y=y)
    d.set(hv, x=CARD_W + 40, y=0) if False else None
    lib.y += CARD_H + 80
    d.C["App card"] = s_id
    d.C["App card/Default"] = c
    d.C["App card/Hover"] = hv
    del sets, parent


def fill_card(d, iid, app, hover=False):
    a = APP[app]
    primary, secondary = actions_for(a)
    d.R("instance.setProperties", {"ids": [iid], "properties": {
        "Version": a["version"], "Opens": opens_label(a), "Tagline": a["tagline"], "Blurb": BLURB[app],
        "Recent": RECENT[app], "Experimental": a["exp"]}})
    d.set(d.sub(iid, "Version"), fill=f"app:{app}/ink")
    wm = d.sub(iid, "Wordmark")
    d.R("instance.setProperties", {"ids": [wm], "properties": {"Name": a["prefix"]}})
    d.set(d.sub(wm, "Craft"), fill=f"app:{app}/ink")
    d.R("image.replace", {"ids": [d.sub(iid, "Icon image")], "path": ICON(app)})
    d.R("instance.setProperties", {"ids": [d.sub(iid, "Primary")], "properties": {"Label": primary}})
    d.R("instance.setProperties", {"ids": [d.sub(iid, "Secondary")], "properties": {"Label": secondary}})
    if hover:
        d.set(d.sub(iid, "Hover rule"), fill=f"app:{app}/color")
        d.R("variable.bind", {"ids": [iid], "field": "fills/1/color", "variable": f"{app}/wash"})


TABLE_COLS = [("App", 300), ("Opens", 250), ("Version", 110), ("Status", 160), ("Recent file", 250), ("", 322)]


def table_row_component(d, lib):
    y = lib.row(62)
    W = sum(w for _, w in TABLE_COLS)
    c = d.comp("Table row", 0, y, W, 62, fill="bg", layout="horizontal", crossAlign="center",
               strokes=[{"type": "solid", "color": "#fff"}], strokeBottomWeight=1, strokeTopWeight=0,
               strokeLeftWeight=0, strokeRightWeight=0, strokeAlign="inside")
    d.R("variable.bind", {"ids": [c], "field": "strokes/0/color", "variable": "line"})

    def col(name, w, **kw):
        return d.frame(c, name, 0, 0, w, 62, fill=None, layout="horizontal", padding=[0, 16, 0, 16], gap=12,
                       crossAlign="center", sizingH="fixed", sizingV="fill", clip=True, **kw)

    a = col("App cell", TABLE_COLS[0][1])
    ic = d.inst("App icon", a, name="Icon")
    d.set(ic, width=36, height=36)
    d.R("instance.setProperties", {"ids": [ic], "properties": {"Shadow": False}})
    names = d.frame(a, "Names", 0, 0, 200, 40, fill=None, layout="vertical", gap=2, sizingH="fill", sizingV="hug")
    d.inst("App wordmark/16", names, name="Wordmark")
    tg = d.text(names, "Writing and document design", "Small/12", "muted", name="Tagline")
    d.set(tg, sizingH="fill", textAutoResize="truncate", maxLines=1)
    o = d.text(col("Opens cell", TABLE_COLS[1][1]), ".DOCX .ODT .MD …", "HUD/11", "muted", name="Opens")
    v = d.text(col("Version cell", TABLE_COLS[2][1]), "0.18.1", "Mono/13", "ink", name="Version")
    d.inst("Status badge", col("Status cell", TABLE_COLS[3][1]), props={"Tone": "Good", "Label": "Ready"},
           name="Status")
    rc = d.text(col("Recent cell", TABLE_COLS[4][1]), "Q3 report.docx · 2 h ago", "Small/12.5", "muted", name="Recent")
    act = col("Actions cell", TABLE_COLS[5][1])
    d.set(act, gap=8, mainAlign="max")
    p = d.inst("Button", act, props={"Kind": "Primary", "Label": "New"}, name="Primary")
    s = d.inst("Button", act, props={"Kind": "Secondary", "Label": "Open file…"}, name="Secondary")
    d.set(p, sizingH="fixed", width=120)
    d.set(s, sizingH="fixed", width=126)
    d.inst("Button/More", act, name="More")
    for lay, name in ((tg, "Tagline"), (o, "Opens"), (v, "Version"), (rc, "Recent")):
        d.prop(c, lay, name)


def fill_row(d, iid, app):
    a = APP[app]
    primary, secondary = actions_for(a)
    d.R("instance.setProperties", {"ids": [iid], "properties": {
        "Tagline": a["tagline"], "Opens": " ".join(e.upper() for e in a["opens"][:3]),
        "Version": a["version"], "Recent": RECENT[app].replace("Recent: ", "").replace("No recent files", "—")}})
    wm = d.sub(iid, "Wordmark")
    d.R("instance.setProperties", {"ids": [wm], "properties": {"Name": a["prefix"]}})
    d.set(d.sub(wm, "Craft"), fill=f"app:{app}/ink")
    d.R("image.replace", {"ids": [d.sub(iid, "Icon image")], "path": ICON(app)})
    st = d.sub(iid, "Status")
    d.R("instance.setProperties", {"ids": [st], "properties": {"Tone": "Warn" if a["exp"] else "Good",
                                                               "Label": "Experimental" if a["exp"] else "Ready"}})
    d.R("instance.setProperties", {"ids": [d.sub(iid, "Primary")], "properties": {"Label": primary}})
    d.R("instance.setProperties", {"ids": [d.sub(iid, "Secondary")], "properties": {"Label": secondary}})
