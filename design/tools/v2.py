"""CraftCloud v2 (2026-11-13): every screen redone in the ArtCraft Launcher's layout language
(storytold/craft-launcher crates/ui-egui), with details from getartcraft.com (headline, meta lines,
icon buttons) and the new brand (mackerel logo, sage accent).

Everything is egui-drawable: hard edges, 1 px hairlines, flat fills, the one dialog/menu shadow,
the app-icon shadow, no rounded corners except app icons and the logo. The one exception, the blur
on switched-off app cards, is a pre-blurred texture in egui (see README).
"""

import os

from lib import APP, ASSETS, ORDER, VERSION, OXICLOUD, DESIGN
from components import ICON, circle_path
from cards import CARD_W, opens_label
from screens import hud, vstack, hstack, side_stroke, screen
from web import tray_icon  # noqa: F401  (kept for the archive)

W, H = 1440, 900
DW, DH = 960, 640
GAP = 160
SERVER_URL = "https://cloud.example.org"
HOST = "cloud.example.org"
OWNER = ("JD", "John Doe", "john", "john@example.org")
USER2 = ("JS", "Jane Smith", "jane", "jane@example.org")
LOGO = os.path.join(DESIGN, "assets", "logo", "craftcloud-logo-256.png")
ACCENT_INK = {False: "#a9c48e", True: "#3b4d2c"}  # literal: FigCraft text ranges can't bind variables
V2_CARD_H = 252

ICONS = {
    "download": "M7 1 L7 9 M3.5 5.5 L7 9 L10.5 5.5 M1.5 12.5 L12.5 12.5",
    "arrow-right": "M1.5 7 L12 7 M8 3 L12 7 L8 11",
    "sync": "M2 6 C2.5 3 5 1.5 7.5 1.8 C9.5 2 11 3.2 11.8 5 M11.8 1.8 L11.8 5 L8.8 5 "
            "M12 8 C11.5 11 9 12.5 6.5 12.2 C4.5 12 3 10.8 2.2 9 M2.2 12.2 L2.2 9 L5.2 9",
    "pause": "M4.5 2 L4.5 12 M9.5 2 L9.5 12",
    "folder": "M1.5 3 L5.5 3 L7 4.5 L12.5 4.5 L12.5 11.5 L1.5 11.5 Z",
    "check": "M2 7.5 L5.5 11 L12 3",
    "external": "M5 2 L12 2 L12 9 M12 2 L3 11",
    "plus": "M7 2 L7 12 M2 7 L12 7",
    "key": circle_path(4.5, 9.5, 2.8, 16) + " M6.5 7.5 L12 2 M10 4 L12 6",
    "logout": "M5.5 2 L2 2 L2 12 L5.5 12 M6 7 L12.5 7 M9.5 4 L12.5 7 L9.5 10",
    "x": "M3 3 L11 11 M11 3 L3 11",
    "save": "M2 2 L10 2 L12 4 L12 12 L2 12 Z M4.5 2 L4.5 5 L9 5 L9 2 M4.5 12 L4.5 8.5 L9.5 8.5 L9.5 12",
    "browser": "M1.5 2.5 L12.5 2.5 L12.5 11.5 L1.5 11.5 Z M1.5 5 L12.5 5",
    "refresh": "M11.5 4.5 C10.5 2.8 8.9 2 7 2 C4.2 2 2 4.2 2 7 C2 9.8 4.2 12 7 12 C9.4 12 11.4 10.3 11.9 8 "
               "M12 1.5 L12 4.8 L8.7 4.8",
}
ICON_BUTTON_KINDS = {"Primary": ("invert_bg", None, "invert_fg"), "Secondary": ("bg", "line_strong", "ink"),
                     "Danger": ("bad", None, "on_bad"), "Accent": ("accent", None, "on_accent")}


# --- components ------------------------------------------------------------------------------------

def v2_components(d, lib):
    y = lib.row(64)
    # Logo: the mackerel squircle when the file is there, else a swappable placeholder.
    c = d.comp("Brand/Logo", 0, y, 64, 64, fill=None, clip=False)
    if os.path.exists(LOGO):
        d.image(c, LOGO, 0, 0, 64, 64, name="Logo image")
    else:
        sq = d.rect(c, 0, 0, 64, 64, "accent", name="Logo placeholder", cornerRadius=14, cornerSmoothing=0.6)
        t = d.text(c, "LOGO", "Badge/10", "on_accent", x=17, y=26, name="Placeholder label")
        del sq, t
    d.set(c, description="CraftCloud logo (mackerel on sage squircle). Placeholder until "
                         "design/assets/logo/craftcloud-mackerel.png exists; rebuild to swap it in.")
    c = d.comp("Brand/Lockup", 120, y, 10, 10, fill=None, layout="horizontal", gap=10, crossAlign="center",
               sizingH="hug", sizingV="hug", clip=False)
    lg = d.inst("Brand/Logo", c, name="Logo")
    d.R("transform.scale", {"ids": [lg], "factor": 24 / 64})
    d.inst("Brand/CraftCloud wordmark", c, name="Wordmark")

    # Icons (14 px, 1.5 px strokes in ink; buttons recolour them).
    y = lib.row(14)
    for i, (n, path) in enumerate(ICONS.items()):
        c = d.comp(f"v2/Icon/{n}", i * 40, y, 14, 14, fill=None, clip=False)
        d.node("vector", c, "glyph", 0, 0, path=path, closed=False, fill=None, stroke="ink",
               strokeWeight=2 if n == "pause" else 1.5)

    # Icon button: Kind × the launcher button (widgets.rs:405), with a leading 14 px icon.
    y = lib.row(34)
    ids = []
    for i, (kind, (fill, stroke, ink)) in enumerate(ICON_BUTTON_KINDS.items()):
        c = d.comp(f"Kind={kind}", i * 200, y, 120, 34, fill=fill, stroke=stroke, layout="horizontal",
                   padding=[0, 16, 0, 14], gap=8, mainAlign="center", crossAlign="center", sizingH="hug",
                   sizingV="fixed")
        ic = d.inst("v2/Icon/plus", c, name="Icon")
        d.set(d.sub(ic, "glyph"), stroke=ink)
        t = d.text(c, "New user", "Button/11", ink, name="Label")
        d.prop(c, t, "Label")
        ids.append(c)
    s = d.R("component.combineVariants", {"ids": ids})["id"]
    d.set(s, name="v2/Icon button", x=0, y=y)
    d.C["v2/Icon button"] = s

    # App toggle card (launcher card, views.rs:554), State = On / Hover / Off.
    y = lib.row(V2_CARD_H)
    on = app_card_comp(d, "State=On", 0, y)
    hv = app_card_comp(d, "State=Hover", CARD_W + 40, y, hover=True)
    off = app_card_comp(d, "State=Off", 2 * (CARD_W + 40), y, off=True)
    s = d.R("component.combineVariants", {"ids": [on, hv, off]})["id"]
    d.set(s, name="v2/App card", x=0, y=y)
    d.C["v2/App card"] = s

    # Headers and footers.
    header_comp(d, lib, "v2/Header/Web", W, ("Home", "Files", "Admin"), web=True)
    header_comp(d, lib, "v2/Header/Desktop", DW, ("Activity", "Folders", "Settings"), web=False)
    footer_comp(d, lib, "v2/Footer/Web", W, f"CraftCloud {VERSION} · {HOST}",
                "Docs ↗   Source ↗   Look based on ArtCraft Launcher by the ArtCraft team")
    footer_comp(d, lib, "v2/Footer/Desktop", DW, f"Synced 2 min ago · {HOST}", "Open in browser ↗")


def app_card_comp(d, name, x, y, hover=False, off=False):
    c = d.comp(name, x, y, CARD_W, V2_CARD_H, fill="bg", stroke="line", layout="vertical", clip=True)
    if hover:
        d.R("fill.add", {"ids": [c], "color": "#3f5fe00d"})
        d.R("variable.bind", {"ids": [c], "field": "fills/1/color", "variable": "wordcraft/wash"})
    rule = d.rect(c, 1, 1, CARD_W - 2, 2, "app:wordcraft/color", name="Hover rule")
    d.absolute(rule, 1, 1)
    d.set(rule, visible=hover)
    strip = d.frame(c, "Strip", 0, 0, CARD_W, 40, fill=None, layout="horizontal", padding=[0, 20, 0, 20],
                    crossAlign="center", sizingH="fill", sizingV="fixed")
    side_stroke(d, strip, bottom=1)
    v = d.text(strip, "0.18.1", "HUD/11", "muted" if off else "app:wordcraft/ink", name="Version")
    d.frame(strip, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    st = d.text(strip, "Off" if off else "In the browser", "HUD/11", "muted" if off else "good", name="State")
    body = d.frame(c, "Body", 0, 0, CARD_W, 100, fill=None, layout="vertical", padding=[18, 20, 20, 20], gap=14,
                   sizingH="fill", sizingV="fill", clip=False)
    content = vstack(d, body, "Content", 0, 0, CARD_W - 40, gap=14)
    d.set(content, sizingH="fill")
    head = d.frame(content, "Head", 0, 0, 300, 56, fill=None, layout="horizontal", gap=14, crossAlign="center",
                   sizingH="fill", sizingV="hug", clip=False)
    slot = d.frame(head, "Icon slot", 0, 0, 56, 56, fill=None, clip=False)
    ic = d.inst("App icon", slot, 0, -4 if hover else 0, name="Icon")
    col = vstack(d, head, "Names", 0, 0, 200, gap=4)
    d.set(col, sizingH="fill")
    wm = d.inst("App wordmark/22", col, name="Wordmark")
    tg = d.text(col, "Writing and document design", "Body/13.5 Tagline", "muted", name="Tagline")
    d.set(tg, sizingH="fill", textAutoResize="truncate", maxLines=1)
    meta = hstack(d, content, "Meta", gap=8)
    exp = d.inst("Status badge", meta, props={"Tone": "Warn", "Label": "Experimental"}, name="Experimental")
    op = d.text(meta, "Opens .DOCX .ODT .MD …", "HUD/11", "muted", name="Opens")
    d.frame(body, "Spacer", 0, 0, 10, 10, fill=None, sizingV="fill")
    tgl = d.frame(body, "Toggle", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug",
                  sizingV="fixed", clip=False)
    d.inst("Cell", tgl, props={"Label": "Off", "Show count": False, "State": "On" if off else "Off"}, name="Cell off")
    d.rect(tgl, 0, 0, 1, 34, "line", name="sep")
    d.inst("Cell", tgl, props={"Label": "On", "Show count": False, "State": "Off" if off else "On"}, name="Cell on")
    if off:
        # Off: grey and blurred (egui: a pre-blurred grey copy of the icon and a 55 % fade, see README).
        d.R("image.adjust", {"ids": [d.sub(ic, "Icon image")], "preset": "blackAndWhite"})
        d.set(content, opacity=0.55)
        d.R("effect.add", {"ids": [content], "effect": {"type": "layerBlur", "radius": 2.5}})
        d.set(d.sub(wm, "Craft"), fill="muted")
    for lay, pn in ((v, "Version"), (tg, "Tagline"), (op, "Opens")):
        d.prop(c, lay, pn)
    d.R("component.addProperty", {"id": c, "name": "Experimental", "type": "boolean", "layer": exp, "default": False})
    d.set(exp, visible=False)
    return c


def fill_v2_card(d, iid, app, state="On"):
    a = APP[app]
    d.R("instance.setProperties", {"ids": [iid], "properties": {
        "State": state, "Version": a["version"], "Tagline": a["tagline"], "Opens": opens_label(a),
        "Experimental": a["exp"]}})
    if state != "Off":
        d.set(d.sub(iid, "Version"), fill=f"app:{app}/ink")
    wm = d.sub(iid, "Wordmark")
    d.R("instance.setProperties", {"ids": [wm], "properties": {"Name": a["prefix"]}})
    if state != "Off":
        d.set(d.sub(wm, "Craft"), fill=f"app:{app}/ink")
    d.R("image.replace", {"ids": [d.sub(iid, "Icon image")], "path": ICON(app)})
    if state == "Off":
        d.R("image.adjust", {"ids": [d.sub(iid, "Icon image")], "preset": "blackAndWhite"})
    if state == "Hover":
        d.set(d.sub(iid, "Hover rule"), fill=f"app:{app}/color")
        d.R("variable.bind", {"ids": [iid], "field": "fills/1/color", "variable": f"{app}/wash"})


def header_comp(d, lib, name, w, nav, web):
    y = lib.row(48)
    h = d.comp(name, 0, y, w, 48, fill="bg", layout="horizontal", crossAlign="center", sizingV="fixed")
    side_stroke(d, h, bottom=1)

    def cell(nm, pad=(0, 16, 0, 16), left=False, right=False, gap=8, fixed=None):
        c = d.frame(h, nm, 0, 0, fixed or 10, 48, fill=None, layout="horizontal", padding=list(pad), gap=gap,
                    crossAlign="center", sizingH="fixed" if fixed else "hug", sizingV="fill", clip=False)
        if fixed:
            d.set(c, mainAlign="center")
        if left or right:
            side_stroke(d, c, left=1 if left else 0, right=1 if right else 0)
        return c

    b = cell("Brand", (0, 20, 0, 20), right=True)
    d.inst("Brand/Lockup", b, name="Lockup")
    if web:
        s = cell("Suite", right=True)
        hud(d, s, "Craft app suite", name="Suite label")
    n = cell("Nav", (0, 12, 0, 12), gap=4, right=True)
    for lab in nav:
        d.inst("Nav cell", n, props={"Label": lab, "State": "On" if lab == nav[0] else "Off"}, name=f"Nav {lab}")
    d.frame(h, "Spacer", 0, 0, 10, 48, fill=None, sizingH="fill")
    if web:
        u = cell("User", left=True, gap=10)
        av = d.frame(u, "Avatar", 0, 0, 26, 26, fill="sunken", stroke="line_strong", layout="horizontal",
                     mainAlign="center", crossAlign="center")
        d.text(av, OWNER[0], "Badge/10", "ink", name="Initials")
        d.text(u, OWNER[1], "Body/13", "ink", name="User name")
        so = cell("Sign out", left=True)
        d.text(so, "Sign out", "HUD/11", "muted", name="Sign out label")
        g = cell("Settings", (0, 0, 0, 0), left=True, fixed=48)
        d.inst("Icon/Settings", g, name="Gear")
    else:
        st = cell("Status", left=True, gap=8)
        d.node("ellipse", st, "Dot", 0, 0, 8, 8, fill="good")
        d.text(st, "Up to date", "Body/13", "ink", name="Status text")
        for k, path in (("Minimize", "M0 5 L10 5"), ("Maximize", "M0 0 L10 0 L10 10 L0 10 Z"),
                        ("Close", "M0 0 L10 10 M10 0 L0 10")):
            c = cell(k, (0, 0, 0, 0), left=(k == "Minimize"), fixed=46)
            d.node("vector", c, "glyph", 0, 0, path=path, closed=(k == "Maximize"), fill=None,
                   stroke="muted", strokeWeight=1)
    return h


def footer_comp(d, lib, name, w, left, right):
    y = lib.row(34)
    f = d.comp(name, 0, y, w, 34, fill="bg", layout="horizontal", padding=[0, 24 if w == W else 20, 0, 24 if w == W
                                                                                          else 20],
               crossAlign="center")
    side_stroke(d, f, top=1)
    lt = d.text(f, left, "HUD/11", "muted", name="Left")
    d.frame(f, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
    rt = d.text(f, right, "Small/12", "muted", name="Right")
    d.prop(f, lt, "Left")
    d.prop(f, rt, "Right")


# --- screen helpers ----------------------------------------------------------------------------------

def page(d, name, x, y, w=W, h=H, light=False, header=True, nav=None, footer_left=None, desktop=False,
         status=None):
    f = screen(d, name, x, y, w, h, light=light)
    if header:
        hid = d.inst("v2/Header/Desktop" if desktop else "v2/Header/Web", f, 0, 0, name="Header")
        navs = ("Activity", "Folders", "Settings") if desktop else ("Home", "Files", "Admin")
        for lab in navs:
            d.R("instance.setProperties", {"ids": [d.sub(hid, f"Nav {lab}")],
                                           "properties": {"State": "On" if lab == nav else "Off"}})
        if nav is None:
            d.set(d.sub(hid, "Nav"), visible=False)
            if not desktop:
                for k in ("User", "Sign out"):
                    d.set(d.sub(hid, k), visible=False)
        if status:
            set_status(d, hid, *status)
    ft = d.inst("v2/Footer/Desktop" if desktop else "v2/Footer/Web", f, 0, h - 34, name="Footer")
    if footer_left:
        d.R("instance.setProperties", {"ids": [ft], "properties": {"Left": footer_left}})
    return f


def set_status(d, hid, color, text):
    d.set(d.sub(hid, "Dot"), fill=color)
    d.set(d.sub(hid, "Status text"), characters=text)


def headline(d, parent, text, style="Display/Hero 56", light=False, x=0, y=0, name="Headline"):
    """Two lines; the word wrapped in *stars* is in accent_ink (a literal per mode: text ranges can't bind)."""
    plain = text.replace("*", "")
    t = d.text(parent, plain, style, "ink_strong", x=x, y=y, name=name)
    a = text.index("*")
    b = text.index("*", a + 1) - 1
    d.R("text.style", {"id": t, "start": a, "end": b, "textFills": [ACCENT_INK[light]]})
    return t


def meta(d, parent, parts, link=None, name="Meta"):
    """Faint mono meta line with middots; `link` (a part) is underlined."""
    r = hstack(d, parent, name, gap=10)
    for i, p in enumerate(parts):
        if i:
            d.text(r, "·", "HUD/11", "muted", name="dot")
        t = d.text(r, p, "HUD/11", "muted" if p != link else "muted", name=p[:30])
        if p == link:
            d.set(t, textDecoration="underline")
    return r


def ibtn(d, parent, label, icon, kind="Primary", name=None, w=None, state=None):
    iid = d.inst("v2/Icon button", parent, props={"Kind": kind, "Label": label}, name=name or label)
    ic = d.sub(iid, "Icon")
    d.R("instance.swap", {"ids": [ic], "component": d.C[f"v2/Icon/{icon}"]})
    ink = ICON_BUTTON_KINDS[kind][2]
    d.set(d.sub(d.sub(iid, "Icon"), "glyph"), stroke=ink)
    if w:
        d.set(iid, sizingH="fixed", width=w)
    if state == "Disabled":
        d.set(iid, opacity=0.5)
    return iid


def btn(d, parent, label, kind="Secondary", name=None, state="Default"):
    return d.inst("Button", parent, props={"Kind": kind, "Label": label, "State": state}, name=name or label)


def split(d, parent, label, icon):
    r = hstack(d, parent, f"Split {label}", gap=1)
    ibtn(d, r, label, icon, "Primary", name="Main")
    ch = d.frame(r, "Chevron part", 0, 0, 34, 34, fill="invert_bg", layout="horizontal", mainAlign="center",
                 crossAlign="center")
    cv = d.inst("Icon/Chevron down", ch, name="Chevron")
    d.set(d.sub(cv, "glyph"), stroke="invert_fg")
    return r


def cells(d, parent, labels, sel=None, name="Cells", disabled=False, counts=None):
    box = d.frame(parent, name, 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed",
                  clip=False)
    for i, lab in enumerate(labels):
        if i:
            d.rect(box, 0, 0, 1, 34, "line", name="sep")
        props = {"Label": lab, "Show count": bool(counts), "State": "On" if lab == sel else "Off"}
        if counts:
            props["Count"] = str(counts[i])
        d.inst("Cell", box, props=props, name=f"{name} {lab}")
    if disabled:
        d.set(box, opacity=0.5)
    return box


def square(d, parent, icon, name):
    """The launcher's refresh square (bar.rs:66): 34 × 34 in a hairline box."""
    s = d.frame(parent, name, 0, 0, 34, 34, fill=None, stroke="line", layout="horizontal", mainAlign="center",
                crossAlign="center")
    ic = d.inst(f"v2/Icon/{icon}", s, name="Glyph")
    d.set(d.sub(ic, "glyph"), stroke="muted")
    return s


def toolbar(d, f, x, y, w):
    return d.frame(f, "Toolbar", x, y, w, 34, fill=None, layout="horizontal", gap=8, crossAlign="center", clip=False)


def spacer(d, parent, v=False):
    return d.frame(parent, "Spacer", 0, 0, 10, 10, fill=None, **({"sizingV": "fill"} if v else {"sizingH": "fill"}))


def panel(d, parent, title, w, gap=8):
    """sheet.rs:93: hairline box, BOX_MARGIN 14 × 12, faint HUD title, 8 px, body; 14 px between panels."""
    p = vstack(d, parent, f"Panel {title}", 0, 0, w, gap=gap, pad=[12, 14, 12, 14])
    d.set(p, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [p], "field": "strokes/0/color", "variable": "line"})
    hud(d, p, title, color="muted", name="Panel title")
    return p


def row(d, parent, label, control_w=200, name=None):
    """settings_dialog.rs:18: label left (Body 14), control right in a fixed-width slot (34 tall)."""
    r = hstack(d, parent, name or f"Row {label}", gap=8, h=34)
    d.set(r, sizingH="fill")
    lt = d.text(r, label, "Body/14", "ink", name="Row label")
    d.set(lt, sizingH="fill")
    slot = d.frame(r, "Control", 0, 0, control_w, 34, fill=None, layout="horizontal", gap=8, crossAlign="center",
                   mainAlign="max", sizingH="fixed", sizingV="fixed", clip=False)
    return r, slot


def note(d, parent, text, w, name="Note", color="muted"):
    t = d.text(parent, text, "Small/12", color, name=name, w=w)
    d.set(t, sizingH="fill")
    return t


def field(d, parent, label, value, state="Default", w=300, hint=None, button=None, placeholder=False):
    fl = vstack(d, parent, f"Field {label}", 0, 0, w, gap=6)
    d.set(fl, sizingH="fill")
    hud(d, fl, label, color="ink", name=f"{label} label")
    r = hstack(d, fl, f"{label} row", gap=8)
    d.set(r, sizingH="fill")
    i = d.inst("Input", r, props={"State": state, "Value": value}, name=f"{label} input")
    d.set(i, sizingH="fill")
    if placeholder:
        d.set(d.sub(i, "Value"), fill="muted")
    if button:
        btn(d, r, button)
    if hint:
        note(d, fl, hint, w, name=f"{label} hint")
    return fl


def scrim(d, f, w, h):
    return d.rect(f, 0, 0, w, h, "scrim", name="Scrim")


def sheet(d, parent, x, y, w, h, title, caption, icon="logo", right=None, name="Sheet"):
    """sheet.rs:21 frame (bg, 1 px line_strong, dialog shadow) + sheet.rs:27 hero (accent wash, 2 px rule)."""
    s = d.frame(parent, name, x, y, w, h, fill="bg", stroke="line_strong", layout="vertical", clip=True)
    d.effect_style(s, "Shadow/Dialog")
    hd = d.frame(s, "Hero", 0, 0, w, 10, fill="accent_tint", layout="horizontal", gap=12, padding=[18, 24, 18, 24],
                 crossAlign="center", sizingH="fill", sizingV="hug", clip=False)
    side_stroke(d, hd, bottom=1)
    rule = d.rect(hd, 0, 0, w, 2, "accent", name="Accent rule")
    d.absolute(rule, 0, 0, w, 2)
    if icon == "logo":
        lg = d.inst("Brand/Logo", hd, name="Hero logo")
        d.R("transform.scale", {"ids": [lg], "factor": 44 / 64})
    elif icon:
        ic = d.inst("App icon", hd, name="Hero icon")
        d.R("image.replace", {"ids": [d.sub(ic, "Icon image")], "path": ICON(icon)})
        d.set(ic, width=44, height=44)
    elif icon is None:
        pass
    col = vstack(d, hd, "Titles", 0, 0, 300, gap=4)
    d.set(col, sizingH="fill")
    d.text(col, title, "Display/Sheet title 26", "ink_strong", name="Sheet title")
    if caption:
        hud(d, col, caption, color="muted", name="Sheet caption")
    if right:
        right(hd)
    body = vstack(d, s, "Sheet body", 0, 0, w, gap=16, pad=[24, 24, 24, 24])
    d.set(body, sizingH="fill", sizingV="fill")
    return s, body


def sheet_actions(d, s, w, left=None, right=None):
    r = d.frame(s, "Sheet actions", 0, 0, w, 62, fill=None, layout="horizontal", gap=8, padding=[14, 24, 14, 24],
                crossAlign="center", sizingH="fill", sizingV="hug", clip=False)
    side_stroke(d, r, top=1)
    if left:
        left(r)
    spacer(d, r)
    if right:
        right(r)
    return r


def steps_cells(d, parent, step, labels):
    box = d.frame(parent, "Steps", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed",
                  clip=False)
    for i, lab in enumerate(labels, 1):
        if i > 1:
            d.rect(box, 0, 0, 1, 34, "line", name="sep")
        c = d.inst("Cell", box, props={"Label": f"{i:02d}" if i != step else f"{i:02d} {lab}", "Show count": False,
                                       "State": "On" if i == step else "Off"}, name=f"Step {i}")
        if i < step:
            d.set(d.sub(c, "Label"), fill="good")
    return box


def avatar(d, parent, ini, size=26):
    av = d.frame(parent, "Avatar", 0, 0, size, size, fill="sunken", stroke="line_strong", layout="horizontal",
                 mainAlign="center", crossAlign="center")
    d.text(av, ini, "Badge/10" if size < 36 else "Mono/13", "ink", name="Initials")
    return av


def logo(d, parent, size, name="Logo", x=0, y=0):
    lg = d.inst("Brand/Logo", parent, x, y, name=name)
    if size != 64:
        d.R("transform.scale", {"ids": [lg], "factor": size / 64})
    return lg


def dot(d, parent, color, size=8):
    return d.node("ellipse", parent, "Dot", 0, 0, size, size, fill=color)


def toast(d, parent, x, y, w, tone, title, msg):
    t = d.frame(parent, f"Toast {title}", x, y, w, 10, fill="bg", stroke="line", layout="horizontal", gap=12,
                padding=[12, 14, 12, 16], crossAlign="center", sizingV="hug", clip=False)
    d.effect_style(t, "Shadow/Dialog")
    bar = d.rect(t, 0, 0, 2, 10, tone, name="Tone rule")
    d.absolute(bar, 0, 0)
    col = vstack(d, t, "Toast text", 0, 0, w - 80, gap=2)
    d.set(col, sizingH="fill")
    d.text(col, title, "Body/14 Medium", "ink_strong", name="Toast title")
    d.text(col, msg, "Small/12.5", "muted", name="Toast message")
    x_ = d.inst("Icon/Close", t, name="Dismiss")
    d.set(d.sub(x_, "glyph"), stroke="muted")
    d.set(bar, x=0, y=0, height=d.R("node.get", {"id": t, "fields": ["height"]})["height"])
    return t


# --- web: setup -------------------------------------------------------------------------------------------

SETUP = ("Welcome", "Owner", "Server", "Web apps", "Done")


def setup_page(d, name, x, y, step, title, caption, sw=880, sh=640):
    f = page(d, name, x, y, nav=None, footer_left=f"CraftCloud {VERSION} · first run")
    scrim(d, f, W, H)
    s, body = sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, title, f"Step {step} of 5 · {caption}",
                    right=lambda hd: steps_cells(d, hd, step, SETUP))
    return f, s, body


def setup_nav(d, s, sw, back=True, primary=("Continue", "arrow-right"), primary_state=None):
    sheet_actions(d, s, sw, left=(lambda r: btn(d, r, "Back")) if back else None,
                  right=lambda r: ibtn(d, r, primary[0], primary[1], state=primary_state))


def v2_setup_1(d, name, x, y):
    sw, sh = 880, 640
    f, s, body = setup_page(d, name, x, y, 1, "Welcome to CraftCloud", "first run")
    r = hstack(d, body, "Intro", gap=28)
    d.set(r, sizingH="fill", crossAlign="min")
    logo(d, r, 128, name="Logo large")
    col = vstack(d, r, "Intro text", 0, 0, 600, gap=14)
    d.set(col, sizingH="fill")
    headline(d, col, "Your files.\nOne *cloud*.", style="Display/Hero 40")
    d.text(col, "CraftCloud keeps your files on this server and in sync on every computer, so the Craft apps "
                "open and save them like any other folder.", "Body/18", "muted", name="Body", w=600)
    meta(d, col, ["Takes about a minute", "Nothing leaves this server", "Setup guide"], link="Setup guide")
    p = panel(d, body, "This server", sw - 48)
    for k, v in (("Address", SERVER_URL), ("Image", f"craftcloud:{VERSION}-apps"), ("Backend", f"OxiCloud {OXICLOUD}"),
                 ("Data", "/app/storage · 1.6 TB free")):
        rr = hstack(d, p, k, gap=0)
        kk = hud(d, rr, k, name="k")
        d.set(kk, sizingH="fixed", width=140)
        d.text(rr, v, "Mono/13", "ink", name="v")
    spacer(d, body, v=True)
    setup_nav(d, s, sw, back=False, primary=("Start setup", "arrow-right"))
    return f


def v2_setup_2(d, name, x, y):
    sw = 880
    f, s, body = setup_page(d, name, x, y, 2, "Create the owner", "owner account")
    d.text(body, "The owner manages people and the server. Only the owner can't be deactivated; you can add admins "
                 "later.", "Body/14", "muted", name="Intro", w=sw - 48)
    two = hstack(d, body, "Two", gap=16)
    d.set(two, sizingH="fill", crossAlign="min")
    a = vstack(d, two, "A", 0, 0, 400, gap=16)
    b = vstack(d, two, "B", 0, 0, 400, gap=16)
    d.set([a, b], sizingH="fill")
    field(d, a, "Username", OWNER[2], w=400, hint="Lower case, no spaces. Used to sign in.")
    field(d, a, "Email", OWNER[3], w=400, hint="Optional. Only shown to admins.")
    field(d, b, "Password", "••••••••••••••••", w=400)
    field(d, b, "Repeat password", "••••••••••••••••", state="Focus", w=400)
    hr = hstack(d, b, "Strength", gap=8)
    d.inst("Status badge", hr, props={"Tone": "Good", "Label": "Strong"}, name="Strength badge")
    d.text(hr, "16 characters · passwords match", "Small/12", "muted", name="Strength text")
    spacer(d, body, v=True)
    setup_nav(d, s, sw)
    return f


def v2_setup_3(d, name, x, y):
    sw = 880
    f, s, body = setup_page(d, name, x, y, 3, "Where is this server?", "server address")
    field(d, body, "Public address", SERVER_URL, w=sw - 48, button="Check",
          hint="Desktop clients and share links use this. Include http:// or https://.")
    r = hstack(d, body, "Result", gap=10)
    d.inst("Status badge", r, props={"Tone": "Good", "Label": "Reachable"}, name="Result badge")
    d.text(r, "Answered in 12 ms · WebDAV OK · plain http", "Small/12.5", "ink", name="Result text")
    w_ = hstack(d, body, "Http warning", gap=10)
    d.inst("Status badge", w_, props={"Tone": "Warn", "Label": "No TLS"}, name="TLS badge")
    d.text(w_, "Fine on a private mesh network. Put it behind https before you open it to the internet.",
           "Small/12.5", "muted", name="TLS text")
    p = panel(d, body, "Storage", sw - 48)
    for k, v in (("Location", "/app/storage (volume craftcloud-data)"), ("Free", "1.6 TB of 2 TB"),
                 ("Versions", "Last 5 per file, in .craftcloud/versions/")):
        rr = hstack(d, p, k, gap=0)
        kk = hud(d, rr, k, name="k")
        d.set(kk, sizingH="fixed", width=140)
        d.text(rr, v, "Mono/13" if k == "Location" else "Body/13", "ink", name="v")
    spacer(d, body, v=True)
    setup_nav(d, s, sw)
    return f


def app_grid(d, parent, w, on_set, hover=None, all_off=False, rows=None):
    """The launcher grid (views.rs:532): cards share 1 px hairlines; columns from CARD_MIN_W 320."""
    cols = max(1, int(w // 320))
    cw = (w + cols - 1) / cols
    ids = ORDER if rows is None else ORDER[:rows * cols]
    g = d.frame(parent, "App grid", 0, 0, w, ((len(ids) + cols - 1) // cols) * (V2_CARD_H - 1) + 1, fill=None,
                clip=False)
    for i, app in enumerate(ids):
        r, c = divmod(i, cols)
        st = "Off" if (all_off or app not in on_set) else ("Hover" if app == hover else "On")
        iid = d.inst("v2/App card", g, c * (cw - 1), r * (V2_CARD_H - 1), name=f"Card {APP[app]['name']}")
        d.set(iid, width=cw)
        fill_v2_card(d, iid, app, st)
    return g


WEB_ON = {"wordcraft", "gridcraft", "deckcraft", "pdfcraft", "figcraft", "photocraft"}


def v2_setup_4(d, name, x, y):
    sw, sh = W - 64, H - 48
    f = page(d, name, x, y, nav=None, footer_left=f"CraftCloud {VERSION} · first run")
    scrim(d, f, W, H)
    s, body = sheet(d, f, 32, 24, sw, sh, "Apps in the browser?", "Step 4 of 5 · web apps · optional",
                    right=lambda hd: steps_cells(d, hd, 4, SETUP))
    d.set(body, gap=14, padding=[18, 24, 0, 24])
    tb = hstack(d, body, "Toolbar", gap=8)
    d.set(tb, sizingH="fill")
    hud(d, tb, "Web apps", color="ink", name="Toggle caption")
    cells(d, tb, ("Off", "On"), sel="On", name="Master")
    cells(d, tb, ("All", "None"), name="Bulk")
    d.text(tb, f"{len(WEB_ON)} of 15 on · click a card to switch it", "Small/12.5", "muted", name="Count")
    spacer(d, tb)
    meta(d, tb, ["-apps image", "about 770 MB download", "2.1 GB on disk"])
    clipf = d.frame(body, "Grid viewport", 0, 0, sw - 48, sh - 220, fill=None, clip=True)
    d.set(clipf, sizingH="fill")
    g = app_grid(d, clipf, sw - 48 - 14, WEB_ON, hover="figcraft")
    del g
    bar = d.rect(clipf, sw - 48 - 10, 0, 10, sh - 220, "accent_tint", name="Scroll track")
    d.rect(clipf, sw - 48 - 10, 0, 10, 180, "accent", name="Scroll bar")
    del bar
    setup_nav(d, s, sw)
    return f


def downloads_panel(d, parent, w, title="Get the desktop client"):
    p = panel(d, parent, title, w, gap=0)
    for i, (osn, label, extra, how) in enumerate((
            ("Linux", "AppImage", "x86_64 · arm64", "Unsigned: chmod +x CraftCloud.AppImage, or nix run."),
            ("macOS", ".dmg", "universal", "Unsigned: right-click the app, Open, then Open again."),
            ("Windows", ".msi", "x64 · arm64", "Unsigned: SmartScreen › More info › Run anyway."))):
        r = hstack(d, p, f"OS {osn}", gap=12, padding=[12, 0, 12, 0])
        d.set(r, sizingH="fill")
        if i:
            side_stroke(d, r, top=1)
        col = vstack(d, r, "Text", 0, 0, 200, gap=3)
        d.set(col, sizingH="fill")
        hh = hstack(d, col, "Head", gap=8)
        d.text(hh, osn, "Body/16 Medium", "ink_strong", name="OS name")
        hud(d, hh, f"{VERSION} · {extra}", color="muted", name="OS meta")
        note(d, col, how, 200, name="How to open")
        if i == 0:
            split(d, r, label, "download")
        else:
            ibtn(d, r, label, "download", "Secondary")
    return p


def v2_setup_5(d, name, x, y):
    sw = 880
    f, s, body = setup_page(d, name, x, y, 5, "Your cloud, ready", "done")
    d.text(body, "Put the desktop client on your computers. It keeps a CraftCloud folder in sync; the Craft apps "
                 "open and save files there.", "Body/14", "muted", name="Body", w=sw - 48)
    downloads_panel(d, body, sw - 48)
    p = panel(d, body, "Connect a device", sw - 48)
    r = hstack(d, p, "URL row", gap=8)
    d.set(r, sizingH="fill")
    i = d.inst("Input", r, props={"Value": SERVER_URL}, name="URL")
    d.set(i, sizingH="fill")
    ibtn(d, r, "Copy", "check", "Secondary")
    spacer(d, body, v=True)
    setup_nav(d, s, sw, back=True, primary=("Go to home", "arrow-right"))
    return f


# --- web: login and device approval -------------------------------------------------------------------------

def v2_login(d, name, x, y, device=False):
    f = page(d, name, x, y, nav=None)
    scrim(d, f, W, H)
    sw, sh = 480, 440 if device else 370
    s, body = sheet(d, f, (W - sw) / 2, (H - sh) / 2 - 10, sw, sh, "Sign in to connect" if device else "Sign in",
                    f"{HOST}")
    if device:
        n = d.frame(body, "Device notice", 0, 0, sw - 48, 10, fill="sunken", stroke="line", layout="horizontal",
                    gap=10, padding=[10, 12, 10, 12], crossAlign="center", sizingH="fill", sizingV="hug")
        ic = d.inst("v2/Icon/browser", n, name="Device icon")
        d.set(d.sub(ic, "glyph"), stroke="accent_ink")
        col = vstack(d, n, "Notice text", 0, 0, 300, gap=2)
        d.set(col, sizingH="fill")
        d.text(col, "Studio wants to connect", "Body/14 Medium", "ink", name="Notice title")
        hud(d, col, "Code 7KQ-4XM · you confirm it next", color="muted", name="Notice code")
    field(d, body, "Username", OWNER[2], state="Focus", w=sw - 48)
    field(d, body, "Password", "••••••••••••", w=sw - 48)
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: d.text(r, "Forgot it? Ask an admin to reset it.", "Small/12", "muted",
                                                  name="Forgot"),
                  right=lambda r: ibtn(d, r, "Sign in", "arrow-right"))
    return f


def v2_device(d, name, x, y, state):
    f = page(d, name, x, y, nav="Home")
    scrim(d, f, W, H)
    sw = 560
    titles = {"code": ("Connect a device", "Enter the code your client shows"),
              "confirm": ("Approve Studio?", "Device request · expires in 9 min"),
              "connected": ("Studio is connected", "Device approved"),
              "denied": ("Request denied", "Device request")}
    sh = {"code": 340, "confirm": 470, "connected": 360, "denied": 330}[state]
    s, body = sheet(d, f, (W - sw) / 2, (H - sh) / 2 - 10, sw, sh, *titles[state])
    if state == "code":
        field(d, body, "Code", "7KQ-4", state="Focus", w=sw - 48,
              hint="Six characters, shown in the CraftCloud window on the computer you're connecting.")
        spacer(d, body, v=True)
        sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"), right=lambda r: ibtn(d, r, "Continue",
                                                                                          "arrow-right"))
    elif state == "confirm":
        p = panel(d, body, "Device", sw - 48)
        for k, v in (("Name", "Studio"), ("System", "Linux · CraftCloud 0.2.0"), ("Asked", "Today 14:30")):
            rr = hstack(d, p, k, gap=0)
            kk = hud(d, rr, k, name="k")
            d.set(kk, sizingH="fixed", width=110)
            d.text(rr, v, "Body/14", "ink", name="v")
        code = d.frame(body, "Code box", 0, 0, sw - 48, 72, fill="sunken", stroke="line", layout="horizontal",
                       mainAlign="center", crossAlign="center", sizingH="fill")
        d.text(code, "7KQ-4XM", "Mono/28 Code", "ink_strong", name="Code")
        note(d, body, "Approve only if this code is on the screen of the computer you're connecting. It gets access "
                      "to all of John Doe's files.", sw - 48)
        spacer(d, body, v=True)
        sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Deny"), right=lambda r: ibtn(d, r, "Approve", "check"))
    elif state == "connected":
        r = hstack(d, body, "State", gap=10)
        d.inst("Status badge", r, props={"Tone": "Good", "Label": "Connected"}, name="Badge")
        hud(d, r, "Today 14:31", color="muted", name="When")
        headline(d, body, "Studio is\n*connected*.", style="Display/Hero 40")
        note(d, body, "You can close this tab: the client takes it from here and starts syncing.", sw - 48)
        spacer(d, body, v=True)
        sheet_actions(d, s, sw, right=lambda r: ibtn(d, r, "Go to home", "arrow-right", "Secondary"))
    else:
        r = hstack(d, body, "State", gap=10)
        d.inst("Status badge", r, props={"Tone": "Bad", "Label": "Denied"}, name="Badge")
        headline(d, body, "Request\n*denied*.", style="Display/Hero 40")
        note(d, body, "Studio didn't get access. If that was a mistake, start again from the client.", sw - 48)
        spacer(d, body, v=True)
        sheet_actions(d, s, sw, right=lambda r: ibtn(d, r, "Go to home", "arrow-right", "Secondary"))
    return f


# --- web: home -------------------------------------------------------------------------------------------

def v2_home(d, name, x, y, light=False):
    f = page(d, name, x, y, light=light, nav="Home")
    top = vstack(d, f, "Hero", 24, 72, 900, gap=16)
    hud(d, top, "01 / Home", color="muted", name="Kicker")
    headline(d, top, "Your *cloud*,\nready.", light=light)
    d.text(top, "Files live on this server and in a CraftCloud folder on each computer. Open and save them with "
                "the Craft apps; changes sync on their own.", "Body/18", "muted", name="Body", w=640)
    meta(d, top, [f"{HOST}", "2 devices", "versions kept: 5", "What syncs"], link="What syncs")
    cols = d.frame(f, "Panels", 24, 372, W - 48, 10, fill=None, layout="horizontal", gap=14, crossAlign="min",
                   sizingV="hug", clip=False)
    cw = (W - 48 - 28) / 3
    left = vstack(d, cols, "Col 1", 0, 0, cw, gap=14)
    mid = vstack(d, cols, "Col 2", 0, 0, cw, gap=14)
    right = vstack(d, cols, "Col 3", 0, 0, cw, gap=14)
    p = panel(d, left, "Storage", cw, gap=10)
    r = hstack(d, p, "Used", gap=10)
    d.text(r, "12.4 GB", "Display/Heading 24", "ink_strong", name="Used big")
    d.text(r, "of 100 GB", "Body/14", "muted", name="Of")
    tr = d.frame(p, "Meter", 0, 0, cw - 28, 4, fill="line", sizingH="fill")
    d.rect(tr, 0, 0, round((cw - 28) * 0.124), 4, "accent", name="Fill")
    meta(d, p, ["3,412 files", "versions 1.1 GB", "trash 0.2 GB"])
    p = panel(d, mid, "Devices", cw, gap=0)
    for i, (dev, osn, seen, tone) in enumerate((("Laptop", "Linux · client 0.1.0", "Now", "good"),
                                                ("Studio", "Linux · client 0.1.0", "2 h ago", "muted"))):
        rr = hstack(d, p, f"Device {dev}", gap=10, padding=[10, 0, 10, 0])
        d.set(rr, sizingH="fill")
        if i:
            side_stroke(d, rr, top=1)
        dot(d, rr, tone)
        col = vstack(d, rr, "t", 0, 0, 200, gap=2)
        d.set(col, sizingH="fill")
        d.text(col, dev, "Body/14 Medium", "ink", name="Device name")
        d.text(col, osn, "Small/12", "muted", name="Device meta")
        hud(d, rr, f"Seen {seen}", color="muted", name="Seen")
    downloads_panel(d, right, cw)
    return f


# --- web: admin -------------------------------------------------------------------------------------------

USERS = [("JD", "John Doe", "john@example.org", "Owner", 12.4, 100, "Active", "Today 18:02"),
         ("JS", "Jane Smith", "jane@example.org", "User", 0.8, 1, "Active", "Yesterday 21:14"),
         ("TA", "Test Account", "test@example.org", "User", 0.0, 1, "Deactivated", "2 Nov 2026")]
TABLE = [("Name", None), ("Role", 176), ("Storage", 150), ("Status", 132), ("Last sign-in", 104), ("", 206)]


def admin_toolbar(d, f, sel, primary):
    tb = toolbar(d, f, 24, 72, W - 48)
    square(d, tb, "refresh", "Refresh")
    cells(d, tb, ("Users", "Server"), sel=sel, name="Sub-nav")
    spacer(d, tb)
    return tb


def users_table(d, f, y, menu_on=None, hover=None):
    tw = W - 48
    t = d.frame(f, "Users table", 24, y, tw, 42 + 62 * len(USERS) + 1, fill="bg", stroke="line", layout="vertical",
                padding=[0, 16, 0, 16], clip=False)
    hd = d.frame(t, "Header row", 0, 0, tw - 32, 42, fill=None, layout="horizontal", crossAlign="center",
                 sizingH="fill")
    side_stroke(d, hd, bottom=1)
    for lab, cw in TABLE:
        c = d.frame(hd, f"H {lab or 'actions'}", 0, 0, cw or 100, 42, fill=None, layout="horizontal", gap=6,
                    crossAlign="center", sizingH="fill" if cw is None else "fixed", sizingV="fill")
        if lab:
            hud(d, c, lab, color="ink" if lab == "Name" else "muted", name="Label")
        if lab == "Name":
            d.inst("Icon/Sort up", c, name="Sort arrow")
    rows = {}
    for i, (ini, nm, mail, role, used, quota, status, last) in enumerate(USERS):
        r = d.frame(t, f"Row {nm}", 0, 0, tw - 32, 62, fill="sunken" if nm == hover else None, layout="horizontal",
                    crossAlign="center", sizingH="fill")
        if i < len(USERS) - 1:
            side_stroke(d, r, bottom=1)
        rows[nm] = r

        def c(cw, nmc):
            return d.frame(r, nmc, 0, 0, cw or 100, 62, fill=None, layout="horizontal", gap=10, crossAlign="center",
                           sizingH="fill" if cw is None else "fixed", sizingV="fill", clip=True)
        a = c(None, "Who")
        avatar(d, a, ini, 38)
        col = vstack(d, a, "Names", 0, 0, 300, gap=2)
        d.text(col, nm, "Body/16 Medium", "ink_strong" if status == "Active" else "muted", name="Name")
        d.text(col, mail, "Small/12", "muted", name="Email")
        b = c(176, "Role")
        d.inst("Status badge", b, props={"Tone": {"Owner": "Accent", "Admin": "Good", "User": "Muted"}[role],
                                         "Label": role}, name="Role tag")
        s_ = c(150, "Storage")
        d.set(s_, layout="vertical", gap=6, mainAlign="center", crossAlign="min")
        d.text(s_, f"{used} of {quota} GB", "Small/12", "ink", name="Used")
        bar = d.frame(s_, "Meter", 0, 0, 120, 4, fill="line")
        if used:
            d.rect(bar, 0, 0, max(2, round(120 * used / quota)), 4, "warn" if used / quota > 0.75 else "accent",
                   name="Fill")
        st = c(132, "Status")
        d.inst("Status badge", st, props={"Tone": "Good" if status == "Active" else "Bad", "Label": status},
               name="Status tag")
        ls = c(104, "Last")
        d.text(ls, last, "Small/12", "muted", name="Last", w=100)
        ac = c(206, "Actions")
        d.set(ac, mainAlign="max", gap=8)
        if status == "Deactivated":
            btn(d, ac, "Activate")
        else:
            btn(d, ac, "Reset password")
        if role == "Owner":
            d.frame(ac, "No menu", 0, 0, 34, 34, fill=None)
        else:
            m = d.inst("Button/More hover" if nm == menu_on else "Button/More", ac, name="More")
            del m
    return t, rows


def v2_admin_users(d, name, x, y, mode="table"):
    f = page(d, name, x, y, nav="Admin")
    tb = admin_toolbar(d, f, "Users", "New user")
    s = d.inst("Search field", tb, props={"Placeholder": "Search people"}, name="Search")
    del s
    hud(d, tb, "Sort", color="muted", name="Sort caption")
    d.inst("Select", tb, props={"Value": "Name"}, name="Sort")
    ibtn(d, tb, "New user", "plus")
    hud(d, f, "3 people · 1 owner · 13.2 GB of 102 GB", 24, 130, color="muted", name="Summary")
    t, rows = users_table(d, f, 154, menu_on="Jane Smith" if mode == "table" else None,
                          hover="Jane Smith" if mode in ("table", "done") else None)
    if mode == "table":
        tb_ = d.R("node.get", {"id": t, "fields": ["absoluteBounds"]})["absoluteBounds"]
        fb = d.R("node.get", {"id": f, "fields": ["absoluteBounds"]})["absoluteBounds"]
        mx = tb_["x"] - fb["x"] + tb_["width"] - 16 - 220
        my = tb_["y"] - fb["y"] + 42 + 62 + 48
        m = d.frame(f, "Row menu", mx, my, 220, 10, fill="bg", stroke="line", layout="vertical", padding=[4, 0, 4, 0],
                    sizingV="hug", clip=False)
        d.effect_style(m, "Shadow/Dialog")
        for lab, hov, tone in (("Details", True, None), ("Reset password…", False, None), ("Make admin", False, None),
                               ("Deactivate", False, "bad")):
            it = d.inst("Menu item", m, props={"Label": lab, "Icon": False, "Show trail": False,
                                               "State": "Hover" if hov else "Default"}, name=f"Item {lab}")
            d.set(it, sizingH="fill")
            if tone:
                d.set(d.sub(it, "Label"), fill=tone)
    if mode == "done":
        toast(d, f, W - 24 - 420, H - 34 - 24 - 70, 420, "good", "Password changed for jane",
              "Tell Jane in person. She picks a new one at her next sign-in.")
    if mode == "new":
        scrim(d, f, W, H)
        sw, sh = 620, 440
        s_, body = sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "New user", "Admin · people")
        two = hstack(d, body, "Two", gap=16)
        d.set(two, sizingH="fill", crossAlign="min")
        a = vstack(d, two, "A", 0, 0, 280, gap=16)
        b = vstack(d, two, "B", 0, 0, 280, gap=16)
        d.set([a, b], sizingH="fill")
        field(d, a, "Username", "maria", state="Focus", w=280)
        field(d, b, "Email", "maria@example.org", w=280)
        field(d, a, "Password", "••••••••••", w=280, hint="They change it at first sign-in.")
        field(d, b, "Storage (GB)", "1", w=280, hint="The default is 1 GB.")
        r, slot = row(d, body, "Role", 220)
        cells(d, slot, ("User", "Admin"), sel="User", name="Role")
        spacer(d, body, v=True)
        sheet_actions(d, s_, sw, left=lambda r: btn(d, r, "Cancel"), right=lambda r: ibtn(d, r, "Create", "plus"))
    if mode == "reset":
        scrim(d, f, W, H)
        sw, sh = 520, 360
        s_, body = sheet(d, f, (W - sw) / 2, (H - sh) / 2, sw, sh, "Reset password", "Jane Smith · jane")
        field(d, body, "New password", "••••••••••••", state="Focus", w=sw - 48,
              hint="At least 10 characters. Jane picks her own at the next sign-in.")
        note(d, body, "Her desktop clients stay connected; only the password changes.", sw - 48)
        spacer(d, body, v=True)
        sheet_actions(d, s_, sw, left=lambda r: btn(d, r, "Cancel"), right=lambda r: ibtn(d, r, "Save", "save"))
    return f


def v2_user_details(d, name, x, y):
    """details.rs:93: a card nearly filling the window (W-64 × H-40) over the scrim."""
    f = page(d, name, x, y, nav="Admin")
    admin_toolbar(d, f, "Users", None)
    users_table(d, f, 154)
    scrim(d, f, W, H)
    pw, ph = W - 64, H - 40
    p = d.frame(f, "Details card", 32, 20, pw, ph, fill="bg", stroke="line_strong", layout="vertical", clip=True)
    d.effect_style(p, "Shadow/Dialog")
    hero = d.frame(p, "Hero", 0, 0, pw, 10, fill="accent_tint", layout="horizontal", gap=18, padding=[24, 24, 24, 24],
                   crossAlign="center", sizingH="fill", sizingV="hug", clip=False)
    side_stroke(d, hero, bottom=1)
    rule = d.rect(hero, 0, 0, pw, 2, "accent", name="Accent rule")
    d.absolute(rule, 0, 0, pw, 2)
    avatar(d, hero, USER2[0], 64)
    col = vstack(d, hero, "Identity", 0, 0, 600, gap=6)
    d.set(col, sizingH="fill")
    hud(d, col, "Person · jane", color="muted", name="Kicker")
    d.text(col, USER2[1], "Display/Page title 32", "ink_strong", name="Name")
    r = hstack(d, col, "Tags", gap=8)
    d.inst("Status badge", r, props={"Tone": "Muted", "Label": "User"}, name="Role")
    d.inst("Status badge", r, props={"Tone": "Good", "Label": "Active"}, name="Status")
    d.text(r, USER2[3], "Small/12.5", "muted", name="Email")
    btn(d, hero, "Reset password")
    btn(d, hero, "Deactivate", kind="Danger")
    cl = d.frame(hero, "Close", 0, 0, 34, 34, fill=None, stroke="line_strong", layout="horizontal",
                 mainAlign="center", crossAlign="center")
    d.inst("Icon/Close", cl, name="Close glyph")
    body = d.frame(p, "Body", 0, 0, pw, 10, fill=None, layout="horizontal", gap=24, padding=[24, 24, 24, 24],
                   sizingH="fill", sizingV="fill", crossAlign="min", clip=False)
    cw = (pw - 48 - 24) / 2
    a = vstack(d, body, "Left", 0, 0, cw, gap=14)
    b = vstack(d, body, "Right", 0, 0, cw, gap=14)
    pa = panel(d, a, "Storage", cw, gap=10)
    rr = hstack(d, pa, "Used", gap=10)
    d.text(rr, "0.8 GB", "Display/Heading 24", "ink_strong", name="Used big")
    d.text(rr, "of 1 GB · 80 %", "Body/14", "warn", name="Of")
    tr = d.frame(pa, "Meter", 0, 0, cw - 28, 4, fill="line", sizingH="fill")
    d.rect(tr, 0, 0, round((cw - 28) * 0.8), 4, "warn", name="Fill")
    rw, slot = row(d, pa, "Quota", 200)
    d.inst("Select", slot, props={"Value": "1 GB"}, name="Quota")
    pd = panel(d, a, "Devices", cw, gap=0)
    for i, (dev, seen) in enumerate((("Jane's laptop", "Yesterday 21:14"),)):
        rr = hstack(d, pd, dev, gap=10, padding=[8, 0, 8, 0])
        d.set(rr, sizingH="fill")
        dot(d, rr, "muted")
        t = d.text(rr, dev, "Body/14", "ink", name="Device")
        d.set(t, sizingH="fill")
        hud(d, rr, seen, color="muted", name="Seen")
        btn(d, rr, "Revoke")
    ps = panel(d, b, "Sign-ins", cw, gap=0)
    for i, (when, where) in enumerate((("Yesterday 21:14", "Desktop client · Jane's laptop"),
                                       ("Yesterday 09:02", "Web · Firefox"), ("10 Nov 2026", "Web · Firefox"))):
        rr = hstack(d, ps, when, gap=10, padding=[8, 0, 8, 0])
        d.set(rr, sizingH="fill")
        if i:
            side_stroke(d, rr, top=1)
        t = d.text(rr, where, "Body/13", "ink", name="Where")
        d.set(t, sizingH="fill")
        d.text(rr, when, "Mono/12", "muted", name="When")
    return f


def v2_admin_server(d, name, x, y, slim=False):
    f = page(d, name, x, y, nav="Admin")
    facts = d.frame(f, "Facts", 24, 72, W - 48, 58, fill=None, stroke="line", layout="horizontal", clip=False)
    for i, (k, v) in enumerate((("CraftCloud", f"{VERSION} · {'slim' if slim else '-apps'} image"),
                                ("OxiCloud", f"{OXICLOUD} · unmodified"), ("Public URL", SERVER_URL),
                                ("Versions kept", "5 per file"))):
        c = vstack(d, facts, f"Fact {k}", 0, 0, 300, gap=4, pad=[10, 16, 10, 16])
        d.set(c, sizingH="fill", sizingV="fill")
        if i:
            side_stroke(d, c, left=1)
        hud(d, c, k, color="muted", name="k")
        d.text(c, v, "Mono/13", "ink", name="v")
    tb = toolbar(d, f, 24, 150, W - 48)
    square(d, tb, "refresh", "Refresh")
    cells(d, tb, ("Users", "Server"), sel="Server", name="Sub-nav")
    hud(d, tb, "Web apps", color="ink", name="Toggle caption")
    cells(d, tb, ("Off", "On"), sel="Off" if slim else "On", name="Master", disabled=slim)
    cells(d, tb, ("All", "None"), name="Bulk", disabled=slim)
    if slim:
        d.inst("Status badge", tb, props={"Tone": "Warn", "Label": "Needs the -apps image"}, name="Needs")
    else:
        d.text(tb, f"{len(WEB_ON)} of 15 on", "Small/12.5", "muted", name="Count")
    spacer(d, tb)
    d.inst("Search field", tb, props={"Placeholder": "Search apps"}, name="Search")
    ibtn(d, tb, "Save", "save", state="Disabled" if slim else None)
    note_text = ("This server runs the slim image: web apps need craftcloud:0.1.0-apps (about 770 MB download, "
                 "2.1 GB on disk)." if slim else
                 "Changes apply immediately. Stored on the server in /app/storage/.craftcloud/settings.env.")
    d.text(f, note_text, "Small/12", "warn" if slim else "muted", x=24, y=196, name="Note")
    vp = d.frame(f, "Grid viewport", 24, 222, W - 48, H - 34 - 222, fill=None, clip=True)
    app_grid(d, vp, W - 48, WEB_ON, hover=None if slim else "gridcraft", all_off=slim)
    return f


# --- desktop -------------------------------------------------------------------------------------------

DESK_FOOT = f"Synced 2 min ago · {HOST}"


def dpage(d, name, x, y, nav="Activity", status=("good", "Up to date"), light=False, h=DH, footer_left=DESK_FOOT):
    return page(d, name, x, y, w=DW, h=h, light=light, nav=nav, desktop=True, status=status,
                footer_left=footer_left)


def signin_sheet(d, name, x, y, step, title, caption, sh):
    f = dpage(d, name, x, y, nav=None, status=("faint", "Not connected"), footer_left="Not connected")
    scrim(d, f, DW, DH)
    sw = 640
    s, body = sheet(d, f, (DW - sw) / 2, 48 + (DH - 48 - 34 - sh) / 2, sw, sh, title, f"Step {step} of 3 · {caption}",
                    right=lambda hd: steps_cells(d, hd, step, ("Server", "Browser", "Folders")))
    return f, s, body, sw


def v2_desktop_connect(d, name, x, y):
    f, s, body, sw = signin_sheet(d, name, x, y, 1, "Connect to CraftCloud", "server", 380)
    field(d, body, "Server address", SERVER_URL, state="Focus", w=sw - 48,
          hint="Include http:// or https://. Your admin has it, or the web app's home page.")
    r = hstack(d, body, "Found", gap=10)
    d.inst("Status badge", r, props={"Tone": "Good", "Label": "Found"}, name="Found badge")
    d.text(r, f"CraftCloud {VERSION} at {HOST}", "Small/12.5", "ink", name="Found text")
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Quit"), right=lambda r: ibtn(d, r, "Continue", "arrow-right"))
    return f


def v2_desktop_browser(d, name, x, y):
    f, s, body, sw = signin_sheet(d, name, x, y, 2, "Continue in your browser", "sign in", 440)
    d.text(body, f"We opened {HOST} in your browser. Sign in there and check it shows this code:", "Body/14", "ink",
           name="Intro", w=sw - 48)
    code = d.frame(body, "Code box", 0, 0, sw - 48, 72, fill="sunken", stroke="line", layout="horizontal",
                   mainAlign="center", crossAlign="center", sizingH="fill")
    d.text(code, "7KQ-4XM", "Mono/28 Code", "ink_strong", name="Code")
    wt = vstack(d, body, "Waiting", 0, 0, sw - 48, gap=6)
    d.set(wt, sizingH="fill")
    hud(d, wt, "Waiting for the browser…", color="accent_ink", name="Waiting label")
    tr = d.frame(wt, "Track", 0, 0, sw - 48, 4, fill="line", sizingH="fill")
    d.rect(tr, 180, 0, 140, 4, "accent", name="Indeterminate")
    meta(d, body, ["This device: Studio", "Linux", "Rename later in Settings"])
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"),
                  right=lambda r: ibtn(d, r, "Open browser again", "external", "Secondary"))
    return f


FOLDERS = [("Documents", "2.1 GB", "1,204", True), ("Design", "840 MB", "212", True),
           ("Photos", "8.9 GB", "1,866", False), ("Projects", "1.2 GB", "96", True), ("Music", "3.4 GB", "410", False),
           ("Shared with me", "340 MB", "58", True), ("Files at the top level", "2 MB", "4", True)]


def folder_rows(d, parent, w, row_h=40, header=True, fill_w=True):
    t = d.frame(parent, "Folder table", 0, 0, w, 10, fill="bg", stroke="line", layout="vertical",
                padding=[0, 16, 0, 16], sizingV="hug", clip=False)
    if fill_w:
        d.set(t, sizingH="fill")
    if header:
        hd = d.frame(t, "Header row", 0, 0, w - 32, 42 if row_h > 40 else 36, fill=None, layout="horizontal",
                     crossAlign="center", sizingH="fill")
        side_stroke(d, hd, bottom=1)
        for lab, cw in (("Cloud folder", None), ("Files", 90), ("Size", 90), ("Sync", 112)):
            c = d.frame(hd, f"H {lab}", 0, 0, cw or 100, 36, fill=None, layout="horizontal", crossAlign="center",
                        sizingH="fill" if cw is None else "fixed", sizingV="fill")
            hud(d, c, lab, color="muted", name="Label")
    for i, (nm, size, files, on) in enumerate(FOLDERS):
        r = d.frame(t, f"Row {nm}", 0, 0, w - 32, row_h, fill=None, layout="horizontal", crossAlign="center",
                    sizingH="fill")
        if i < len(FOLDERS) - 1:
            side_stroke(d, r, bottom=1)
        a = d.frame(r, "Name", 0, 0, 100, row_h, fill=None, layout="horizontal", gap=10, crossAlign="center",
                    sizingH="fill", sizingV="fill")
        if nm.startswith("Files at"):
            ic = d.inst("Icon/File", a, name="Icon")
        else:
            ic = d.inst("Icon/Folder", a, name="Icon")
        del ic
        d.text(a, nm, "Body/14", "ink" if on else "muted", name="Folder")
        for v, cw, nmc in ((files, 90, "Files"), (size, 90, "Size")):
            c = d.frame(r, nmc, 0, 0, cw, row_h, fill=None, layout="horizontal", crossAlign="center",
                        sizingH="fixed", sizingV="fill")
            d.text(c, v, "Mono/12", "muted", name=nmc)
        c = d.frame(r, "Sync", 0, 0, 112, row_h, fill=None, layout="horizontal", crossAlign="center",
                    sizingH="fixed", sizingV="fill")
        cells(d, c, ("Off", "On"), sel="On" if on else "Off", name="Toggle")
    return t


def v2_desktop_choose(d, name, x, y):
    f = dpage(d, name, x, y, nav=None, status=("accent_ink", "Signed in"), footer_left=f"Signed in as john · {HOST}")
    scrim(d, f, DW, DH)
    sw, sh = DW - 64, DH - 48 - 34 - 16
    s, body = sheet(d, f, 32, 48 + 8, sw, sh, "Choose what to sync", "Step 3 of 3 · folders",
                    right=lambda hd: steps_cells(d, hd, 3, ("Server", "Browser", "Folders")))
    d.set(body, gap=12, padding=[16, 24, 16, 24])
    r, slot = row(d, body, "~/CraftCloud", 230, name="Local folder")
    d.set(d.sub(r, "Row label"), characters="Local folder  ~/CraftCloud · 212 GB free")
    btn(d, slot, "Change…")
    folder_rows(d, body, sw - 48, row_h=34, header=False)
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: d.text(r, "Downloads 4.5 GB · 1,574 files", "Small/12.5", "muted",
                                                  name="Total"),
                  right=lambda r: ibtn(d, r, "Start syncing", "sync"))
    return f


ACT = [("14:32", "Up", "Good", "Documents/Reports/Q3 report.docx", "248 KB"),
       ("14:32", "Up", "Good", "Notes/Ideas.md", "3 KB"),
       ("14:20", "Down", "Accent", "Design/Poster.psd", "42.1 MB · from Laptop"),
       ("13:58", "Conflict", "Warn", "Finance/Budget 2027.xlsx", "kept both"),
       ("13:41", "Deleted", "Bad", "Finance/Old budget.xlsx", "to the cloud trash"),
       ("12:05", "Down", "Accent", "Photos/2026/IMG_2041.CR3", "28.6 MB"),
       ("11:47", "Up", "Good", "Projects/Studio/Floorplan.dwg", "3.4 MB")]


def activity_list(d, parent, x, y, w, rows=ACT, h=None):
    t = d.frame(parent, "Activity list", x, y, w, 10, fill="bg", stroke="line", layout="vertical",
                padding=[0, 16, 0, 16], sizingV="hug", clip=True)
    for i, (tm, kind, tone, path, extra) in enumerate(rows):
        r = d.frame(t, f"Row {i}", 0, 0, w - 32, 40, fill=None, layout="horizontal", gap=14, crossAlign="center",
                    sizingH="fill")
        if i < len(rows) - 1:
            side_stroke(d, r, bottom=1)
        d.text(r, tm, "Mono/12", "muted", name="Time")
        bd = d.inst("Status badge", r, props={"Tone": tone, "Label": kind}, name="Kind")
        d.set(bd, sizingH="fixed", width=84)
        p = d.text(r, path, "Body/13", "ink", name="Path")
        d.set(p, sizingH="fill")
        d.text(r, extra, "Small/12", "muted", name="Extra")
    return t


def v2_desktop_activity(d, name, x, y, light=False, syncing=False):
    st = ("accent_ink", "Syncing 3 of 12") if syncing else ("good", "Up to date")
    f = dpage(d, name, x, y, light=light, status=st,
              footer_left=f"Syncing · {HOST}" if syncing else DESK_FOOT)
    top = vstack(d, f, "Hero", 24, 72, DW - 48, gap=12)
    if syncing:
        headline(d, top, "Syncing\n*3 of 12* files.", style="Display/Hero 40", light=light)
    else:
        headline(d, top, "Everything\nis *synced*.", style="Display/Hero 40", light=light)
    meta(d, top, ["~/CraftCloud", HOST, "3,412 files", "Open folder"], link="Open folder")
    if syncing:
        pr = vstack(d, top, "Progress", 0, 0, DW - 48, gap=6)
        d.set(pr, sizingH="fill")
        d.text(pr, "Design/Poster.psd · 41 MB of 58 MB", "Small/12", "ink", name="Caption")
        tr = d.frame(pr, "Track", 0, 0, DW - 48, 4, fill="line", sizingH="fill")
        d.rect(tr, 0, 0, round((DW - 48) * 0.7), 4, "accent", name="Fill")
    r = hstack(d, top, "Actions", gap=8)
    ibtn(d, r, "Sync now", "sync")
    ibtn(d, r, "Resume" if False else "Pause", "pause", "Secondary")
    ibtn(d, r, "Open folder", "folder", "Secondary")
    ty = 72 + d.R("node.get", {"id": top, "fields": ["height"]})["height"] + 20
    hud(d, f, "Activity · today", 24, ty, color="muted", name="List label")
    activity_list(d, f, 24, ty + 22, DW - 48, ACT[:max(3, int((DH - 34 - 16 - ty - 22) // 40))])
    return f


def v2_desktop_conflict(d, name, x, y):
    f = v2_desktop_activity(d, name, x, y)
    d.set(d.sub(d.sub(f, "Header"), "Dot"), fill="warn")
    d.set(d.sub(d.sub(f, "Header"), "Status text"), characters="1 conflict")
    scrim(d, f, DW, DH)
    sw, sh = 720, 470
    s, body = sheet(d, f, (DW - sw) / 2, 48 + 12, sw, sh, "Budget 2027.xlsx changed in two places", "Conflict",
                    icon="gridcraft")
    d.text(body, "It was saved on this computer and on Laptop before either change reached the other. Nothing is lost.",
           "Body/14", "muted", name="Intro", w=sw - 48)
    two = d.frame(body, "Versions", 0, 0, sw - 48, 10, fill=None, layout="horizontal", gap=14, sizingH="fill",
                  sizingV="hug", crossAlign="min", clip=False)
    for lab, dev, when, size in (("Cloud", "Laptop", "Today 13:56", "88 KB"),
                                 ("This computer", "Studio", "Today 13:58", "91 KB")):
        p = panel(d, two, lab, (sw - 62) / 2, gap=6)
        d.text(p, dev, "Body/16 Medium", "ink_strong", name="Device")
        meta(d, p, [f"Saved {when}", size])
    nb = vstack(d, body, "Keep both explain", 0, 0, sw - 48, gap=4, pad=[10, 12, 10, 12], fill="sunken")
    d.set(nb, sizingH="fill")
    d.text(nb, "Keep both leaves the cloud file as it is and saves yours next to it as", "Small/12.5", "ink",
           name="Explain")
    d.text(nb, "Budget 2027 (conflict 2026-11-13 13.58 Studio).xlsx", "Mono/12", "ink_strong", name="Copy name")
    note(d, body, "Keep mine or Keep cloud replaces the other version; the replaced file goes to .craftcloud/versions/.",
         sw - 48)
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Decide later"),
                  right=lambda r: (btn(d, r, "Keep cloud"), btn(d, r, "Keep mine"), ibtn(d, r, "Keep both", "check")))
    return f


def v2_desktop_folders(d, name, x, y):
    f = dpage(d, name, x, y, nav="Folders")
    tb = toolbar(d, f, 24, 72, DW - 48)
    square(d, tb, "refresh", "Refresh")
    hud(d, tb, "~/CraftCloud · 212 GB free", color="muted", name="Local")
    spacer(d, tb)
    hud(d, tb, "Sort", color="muted", name="Sort caption")
    d.inst("Select", tb, props={"Value": "Name"}, name="Sort")
    btn(d, tb, "Revert")
    ibtn(d, tb, "Apply", "check")
    t = folder_rows(d, f, DW - 48, fill_w=False)
    d.set(t, x=24, y=124)
    nt = d.text(f, "Unticking a folder removes its local copy from this computer; the cloud keeps it. Changes take effect "
               "when you apply them.", "Small/12", "muted", name="Note", w=DW - 48)
    d.set(nt, x=24, y=124 + d.R("node.get", {"id": t, "fields": ["height"]})["height"] + 12)
    return f


def v2_desktop_settings(d, name, x, y):
    f = v2_desktop_activity(d, name, x, y)
    hid = d.sub(f, "Header")
    for lab in ("Activity", "Settings"):
        d.R("instance.setProperties", {"ids": [d.sub(hid, f"Nav {lab}")],
                                       "properties": {"State": "On" if lab == "Settings" else "Off"}})
    scrim(d, f, DW, DH)
    sw, sh = DW - 64, DH - 48
    s, body = sheet(d, f, 32, 24, sw, sh, "Settings", f"CraftCloud {VERSION} · Studio")
    d.set(body, layout="horizontal", gap=24, crossAlign="min")
    cw = (sw - 48 - 24) / 2
    a = vstack(d, body, "Left", 0, 0, cw, gap=14)
    b = vstack(d, body, "Right", 0, 0, cw, gap=14)
    p = panel(d, a, "Account", cw)
    for k, v in (("Server", HOST), ("User", "John Doe · john")):
        rr = hstack(d, p, k, gap=0)
        kk = hud(d, rr, k, color="muted", name="k")
        d.set(kk, sizingH="fixed", width=90)
        d.text(rr, v, "Body/14", "ink", name="v")
    r, slot = row(d, p, "This device", 200)
    i = d.inst("Input", slot, props={"Value": "Studio"}, name="Device name")
    d.set(i, sizingH="fill")
    ibtn(d, p, "Sign out", "logout", "Secondary")
    p = panel(d, b, "On this computer", cw)
    r, slot = row(d, p, "Start at login", 130)
    cells(d, slot, ("Off", "On"), sel="On", name="Login")
    r, slot = row(d, p, "Syncing", 130)
    cells(d, slot, ("Paused", "Active"), sel="Active", name="Syncing")
    r, slot = row(d, p, "Appearance", 200)
    cells(d, slot, ("System", "Dark", "Light"), sel="System", name="Theme")
    p = panel(d, b, "About", cw)
    meta(d, p, [f"CraftCloud {VERSION}", "Linux x86_64", "MIT OR Apache-2.0"])
    meta(d, p, ["Source", "Release notes", "Report a problem"], link="Source")
    note(d, p, "Look based on ArtCraft Launcher by the ArtCraft team.", cw - 28)
    return f


def v2_desktop_states(d, name, x, y):
    f = screen(d, name, x, y, DW, DH)
    hud(d, f, "Compact states · inside the Activity tab, under the headline", 24, 20, color="muted", name="Label")
    cw, ch = (DW - 48 - 28) / 3, 270
    states = [
        ("Muted", "Paused", "Syncing is paused", "Paused by you at 13:10. 12 changes are waiting.",
         [("Primary", "Resume", "sync")]),
        ("Bad", "Offline", "Server unreachable", f"{HOST} hasn't answered since 14:02. Local changes are safe; the "
                                                 "client retries every minute.", [("Secondary", "Retry now", "refresh")]),
        ("Warn", "Signed out", "Sign in again", "The session ended (password changed or the device was revoked). "
                                                "Nothing was deleted.", [("Primary", "Sign in again", "browser")]),
        ("Warn", "Mass delete", "This would delete 42 files", "42 files vanished from ~/CraftCloud/Photos at once. "
                                                              "Sync is held until you decide.",
         [("Danger", "Delete them", "x"), ("Secondary", "Keep", "check")]),
        ("Bad", "Folder missing", "~/CraftCloud is gone", "The local folder was moved or deleted. Point the client at "
                                                          "it again, or download a fresh copy.",
         [("Secondary", "Locate…", "folder"), ("Primary", "Download again", "download")]),
    ]
    for i, (tone, badge, title, text, bts) in enumerate(states):
        r_, c_ = divmod(i, 3)
        p = d.frame(f, f"State {badge}", 24 + c_ * (cw + 14), 48 + r_ * (ch + 14), cw, ch, fill="bg", stroke="line",
                    layout="vertical", gap=10, padding=[12, 14, 14, 14], clip=True)
        d.inst("Status badge", p, props={"Tone": tone, "Label": badge}, name="Badge")
        d.text(p, title, "Body/16 Medium", "ink_strong", name="Title", w=cw - 28)
        note(d, p, text, cw - 28)
        spacer(d, p, v=True)
        rr = vstack(d, p, "Buttons", 0, 0, cw - 28, gap=8)
        for kind, lab, icon in bts:
            bi = ibtn(d, rr, lab, icon, kind)
            d.set(bi, sizingH="fill")
    # The sixth cell: the header status cell in each state.
    p = d.frame(f, "Status cell states", 24 + 2 * (cw + 14), 48 + ch + 14, cw, ch, fill="bg", stroke="line",
                layout="vertical", gap=12, padding=[12, 14, 14, 14], clip=True)
    hud(d, p, "Header status cell", color="muted", name="T")
    for color, text in (("good", "Up to date"), ("accent_ink", "Syncing 3 of 12"), ("muted", "Paused"),
                        ("bad", "Offline"), ("warn", "1 conflict"), ("faint", "Not connected")):
        rr = hstack(d, p, text, gap=8)
        dot(d, rr, color)
        d.text(rr, text, "Body/13", "ink", name="t")
    return f


def logo_state(d, parent, state, size=22, name=None):
    box = d.frame(parent, name or f"Tray {state}", 0, 0, size + 4, size + 4, fill=None, clip=False)
    logo(d, box, size, x=0, y=0)
    if state:
        col = {"Synced": "good", "Syncing": "accent_ink", "Paused": "warn", "Error": "bad"}[state]
        d.node("ellipse", box, "Status dot", size - 6, size - 6, 10, 10, fill=col, stroke="bg", strokeWeight=1.5,
               strokeAlign="outside")
    return box


def v2_desktop_tray(d, name, x, y):
    """The tray as apps/craftcloud/src/tray.rs builds it: a disabled status line ("CraftCloud · " and the header's
    status text), Open CraftCloud folder, Open in browser, Sync now, Pause / Resume syncing, a Recent files submenu
    (the last 5 file names, newest first), then Show CraftCloud, Settings… and Quit CraftCloud. No accelerators."""
    w, h = 560, 1000
    f = screen(d, name, x, y, w, h)
    d.set(f, fill="sunken")
    bar = d.frame(f, "System tray", 0, 0, w, 32, fill="bg", layout="horizontal", padding=[0, 12, 0, 12], gap=10,
                  crossAlign="center", mainAlign="max")
    side_stroke(d, bar, bottom=1)
    d.text(bar, "Fri 13 Nov  14:33", "Small/12", "ink", name="Clock")
    sel = d.frame(bar, "Tray icon (open)", 0, 0, 30, 30, fill="line", layout="horizontal", mainAlign="center",
                  crossAlign="center", clip=False)
    logo_state(d, sel, "Synced", 20)

    def menu(name_, x_, y_, mw):
        m = d.frame(f, name_, x_, y_, mw, 10, fill="bg", stroke="line", layout="vertical", padding=[4, 0, 4, 0],
                    sizingV="hug", clip=False)
        d.effect_style(m, "Shadow/Dialog")
        return m

    def item(m, lab, trail=None, hover=False, disabled=False):
        it = d.inst("Menu item", m, props={"Label": lab, "Icon": False, "Show trail": bool(trail),
                                           "State": "Hover" if hover else "Default", **({"Trail": trail} if trail
                                                                                       else {})}, name=f"Item {lab}")
        d.set(it, sizingH="fill")
        if disabled:
            d.set(d.sub(it, "Label"), fill="muted")

    def sep(m, mw):
        s = d.frame(m, "Separator", 0, 0, mw, 9, fill=None, layout="vertical", padding=[4, 0, 4, 0], sizingH="fill")
        rr = d.rect(s, 0, 0, mw, 1, "line", name="rule")
        d.set(rr, sizingH="fill")

    def full(m, mw, line="Up to date", paused=False, missing=False, hover=None):
        item(m, f"CraftCloud · {line}", disabled=True)
        sep(m, mw)
        item(m, "Open CraftCloud folder", disabled=missing)
        item(m, "Open in browser")
        item(m, "Sync now", disabled=missing)
        item(m, "Resume syncing" if paused else "Pause syncing")
        item(m, "Recent files", "›", hover=hover == "recent")
        sep(m, mw)
        item(m, "Show CraftCloud")
        item(m, "Settings…")
        item(m, "Quit CraftCloud")

    # The open menu (right click; on macOS any click), with Recent files open to its left.
    mw, sw = 250, 200
    mx = w - 12 - mw
    full(menu("Tray menu", mx, 38, mw), mw, hover="recent")
    sm = menu("Recent files submenu", mx - sw + 2, 38 + 4 + 32 + 9 + 4 * 32 - 4, sw)
    for i, fn in enumerate(("Q3 report.docx", "Ideas.md", "Poster.psd", "Budget 2027.xlsx", "Floorplan.png")):
        item(sm, fn, hover=i == 0)
    d.text(f, "Left click opens the window; right click opens the menu (macOS: any click). Recent files: the "
              "last 5 that went up, came down or moved, newest first; a pick opens the file in its app. An empty "
              "slot shows \u2014.", "Small/12", "muted", x=24, y=52, w=mx - 24 - 24, name="How it opens")
    # The same menu while paused and while the folder is missing.
    yy = 388
    for i, (lab, kw) in enumerate((("Paused", {"line": "Paused", "paused": True}),
                                   ("Folder missing", {"line": "Folder missing", "missing": True}))):
        xx = 24 + i * 262
        hud(d, f, lab, xx, yy, color="muted", name=f"Variant {i}")
        full(menu(f"Menu {lab}", xx, yy + 22, 250), 250, **kw)
    st = d.frame(f, "Icon states", 24, h - 190, w - 48, 100, fill="bg", stroke="line", layout="horizontal",
                 clip=False)
    for i, (s_, lab) in enumerate((("Synced", "Up to date"), ("Syncing", "Syncing"), ("Paused", "Paused"),
                                   ("Error", "Error"))):
        c = vstack(d, st, f"State {lab}", 0, 0, (w - 48) / 4, gap=8, pad=[14, 0, 14, 0])
        d.set(c, sizingH="fill", sizingV="fill", crossAlign="center")
        if i:
            side_stroke(d, c, left=1)
        logo_state(d, c, s_, 22)
        hud(d, c, lab, color="ink", name="State label")
    hud(d, f, "Tray icon: the logo with a status dot", 24, h - 258, color="muted", name="States label")
    d.text(f, "Paused also stands for signed out and waiting for you (mass delete); Error for offline, folder "
              "missing and a failed sync.", "Small/12", "muted", x=24, y=h - 238, w=w - 48, name="States note")
    wi = d.frame(f, "Window icon", 24, h - 80, w - 48, 64, fill=None, layout="horizontal", gap=16,
                 crossAlign="center", clip=False)
    hud(d, wi, "Window icon", color="muted", name="WI label")
    for sz in (48, 32, 16):
        logo(d, wi, sz, name=f"Icon {sz}")
    return f


# --- build ----------------------------------------------------------------------------------------------

def build_v2(d):
    frames = []

    def add(n, fid):
        frames.append((n, fid))

    d.page("v2 · Web")
    col = W + GAP
    rowh = H + GAP
    web = [
        ("v2-web-setup-1", lambda n, x, y: v2_setup_1(d, n, x, y)),
        ("v2-web-setup-2", lambda n, x, y: v2_setup_2(d, n, x, y)),
        ("v2-web-setup-3", lambda n, x, y: v2_setup_3(d, n, x, y)),
        ("v2-web-setup-4", lambda n, x, y: v2_setup_4(d, n, x, y)),
        ("v2-web-setup-5", lambda n, x, y: v2_setup_5(d, n, x, y)),
        ("v2-web-login", lambda n, x, y: v2_login(d, n, x, y)),
        ("v2-web-login-device", lambda n, x, y: v2_login(d, n, x, y, device=True)),
        ("v2-web-device-code", lambda n, x, y: v2_device(d, n, x, y, "code")),
        ("v2-web-device", lambda n, x, y: v2_device(d, n, x, y, "confirm")),
        ("v2-web-device-connected", lambda n, x, y: v2_device(d, n, x, y, "connected")),
        ("v2-web-device-denied", lambda n, x, y: v2_device(d, n, x, y, "denied")),
        ("v2-web-home", lambda n, x, y: v2_home(d, n, x, y)),
        ("v2-web-home-light", lambda n, x, y: v2_home(d, n, x, y, light=True)),
        ("v2-web-admin-users", lambda n, x, y: v2_admin_users(d, n, x, y)),
        ("v2-web-admin-users-new", lambda n, x, y: v2_admin_users(d, n, x, y, mode="new")),
        ("v2-web-admin-users-reset", lambda n, x, y: v2_admin_users(d, n, x, y, mode="reset")),
        ("v2-web-admin-users-reset-done", lambda n, x, y: v2_admin_users(d, n, x, y, mode="done")),
        ("v2-web-admin-user-details", lambda n, x, y: v2_user_details(d, n, x, y)),
        ("v2-web-admin-server", lambda n, x, y: v2_admin_server(d, n, x, y)),
        ("v2-web-admin-server-slim", lambda n, x, y: v2_admin_server(d, n, x, y, slim=True)),
    ]
    for i, (n, fn) in enumerate(web):
        r, c = divmod(i, 4)
        add(n, fn(n, c * col, r * rowh))

    d.page("v2 · Desktop")
    col = DW + 120
    rowh = DH + 120
    desk = [
        ("v2-desktop-connect", v2_desktop_connect), ("v2-desktop-browser", v2_desktop_browser),
        ("v2-desktop-choose", v2_desktop_choose), ("v2-desktop-activity", v2_desktop_activity),
        ("v2-desktop-activity-light", lambda d_, n, x, y: v2_desktop_activity(d_, n, x, y, light=True)),
        ("v2-desktop-activity-syncing", lambda d_, n, x, y: v2_desktop_activity(d_, n, x, y, syncing=True)),
        ("v2-desktop-conflict", v2_desktop_conflict), ("v2-desktop-folders", v2_desktop_folders),
        ("v2-desktop-settings", v2_desktop_settings), ("v2-desktop-states", v2_desktop_states),
        ("v2-desktop-tray", v2_desktop_tray),
    ]
    for i, (n, fn) in enumerate(desk):
        r, c = divmod(i, 4)
        add(n, fn(d, n, c * col, r * rowh))
    return frames
