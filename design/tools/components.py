"""The CraftCloud component library, built in FigCraft (page "Components")."""

import math
import os

from lib import APP, ASSETS, USER, STORAGE, paint

ICON = lambda app: os.path.join(ASSETS, "icons", f"{app}.png")  # noqa: E731


def gear_path(cx, cy, r_out, r_in, r_hole, teeth=8):
    """A gear outline (clockwise) with a round hole (counter-clockwise), so the nonzero rule cuts it."""
    pts = []
    n = teeth * 4
    for i in range(n):
        a = 2 * math.pi * i / n - math.pi / 2
        r = r_out if (i % 4) in (0, 1) else r_in
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    d = "M" + " L".join(f"{x:.2f} {y:.2f}" for x, y in pts) + " Z "
    hole = []
    for i in range(24):
        a = -2 * math.pi * i / 24
        hole.append((cx + r_hole * math.cos(a), cy + r_hole * math.sin(a)))
    d += "M" + " L".join(f"{x:.2f} {y:.2f}" for x, y in hole) + " Z"
    return d


def circle_path(cx, cy, r, n=28):
    pts = [(cx + r * math.cos(2 * math.pi * i / n), cy + r * math.sin(2 * math.pi * i / n)) for i in range(n)]
    return "M" + " L".join(f"{x:.2f} {y:.2f}" for x, y in pts) + " Z"


class Library:
    def __init__(self, d):
        self.d = d
        self.y = 0  # next row on the Components page

    def row(self, h):
        y = self.y
        self.y += h + 60
        return y

    # --- icons ---------------------------------------------------------------------------------
    def icons(self):
        d = self.d
        y = self.row(24)
        x = 0

        def icon(name, w=16, h=16):
            nonlocal x
            c = d.comp(name, x, y, w, h, fill=None, clip=False)
            x += w + 40
            return c

        c = icon("Icon/Settings", 18, 18)
        d.node("vector", c, "glyph", 0, 0, path=gear_path(9, 9, 8.5, 6.4, 2.6), fill="muted", stroke=None)
        c = icon("Icon/Search")
        d.node("vector", c, "glyph", 0, 0, path=circle_path(6.5, 6.5, 5) + " M10.2 10.2 L14.5 14.5", closed=False,
               fill=None, stroke="muted", strokeWeight=1.5)
        c = icon("Icon/Grid")
        for i, (gx, gy) in enumerate([(1, 1), (9, 1), (1, 9), (9, 9)]):
            d.rect(c, gx, gy, 6, 6, "muted", name=f"cell {i}")
        c = icon("Icon/List")
        for i in range(3):
            d.rect(c, 1, 2 + i * 5, 14, 2, "muted", name=f"line {i}")
        c = icon("Icon/Chevron down", 10, 10)
        d.node("vector", c, "glyph", 1, 3, path="M0 0 L4 4 L8 0", closed=False, fill=None, stroke="muted", strokeWeight=1.5)
        c = icon("Icon/Close", 12, 12)
        d.node("vector", c, "glyph", 1, 1, path="M0 0 L10 10 M10 0 L0 10", closed=False, fill=None, stroke="ink",
               strokeWeight=1.5)
        c = icon("Icon/Sort up", 8, 8)
        d.node("vector", c, "glyph", 0, 1.5, path="M0 5 L4 0 L8 5 Z", fill="ink", stroke=None)
        c = icon("Icon/Check", 12, 12)
        d.node("vector", c, "glyph", 1, 2, path="M0 4 L3.5 7.5 L10 0", closed=False, fill=None, stroke="ink",
               strokeWeight=1.75)
        c = icon("Icon/Star", 16, 16)
        d.node("star", c, "glyph", 0, 0, 16, 16, pointCount=5, innerRadius=0.45, fill="warn", stroke=None)
        c = icon("Icon/Cloud", 22, 22)
        d.node("vector", c, "glyph", 1, 4, path="M4 14 C0.5 14 0 9 3.5 8.5 C4 4.5 9 3 11 6.5 C13.5 4.5 17 6 16.5 9 "
               "C20 9 20 14 16.5 14 Z", fill="ink", stroke=None)
        c = icon("Icon/Menu", 18, 18)
        for i in range(3):
            d.rect(c, 1, 3 + i * 5.5, 16, 1.5, "ink", name=f"bar {i}")
        c = icon("Icon/Folder", 20, 20)
        d.node("vector", c, "glyph", 1, 3, path="M0 2 L0 14 L18 14 L18 4 L8.5 4 L6.5 1 L0 1 Z", fill="muted", stroke=None)
        # A page with a folded corner; its fill is overridden per file type with the app's colour.
        c = icon("Icon/File", 20, 20)
        d.node("vector", c, "page", 3, 1, path="M0 0 L9.5 0 L14 4.5 L14 18 L0 18 Z", fill="muted", stroke=None)
        d.node("vector", c, "fold", 12.5, 1, path="M0 0 L4.5 4.5 L0 4.5 Z", fill="#ffffff80", stroke=None)
        c = icon("Icon/Lock", 14, 14)
        d.rect(c, 2, 6, 10, 7, "muted", name="body")
        d.node("vector", c, "shackle", 4, 1, path="M0 5 L0 3 C0 0.5 6 0.5 6 3 L6 5", closed=False, fill=None,
               stroke="muted", strokeWeight=1.5)

    # --- small parts ---------------------------------------------------------------------------
    def wordmark(self):
        """The placeholder text wordmark: swap this one component for the logo."""
        d = self.d
        y = self.row(30)
        c = d.comp("Brand/CraftCloud wordmark", 0, y, 10, 10, layout="horizontal", sizingH="hug", sizingV="hug",
                   crossAlign="baseline")
        d.text(c, "Craft", "Display/Wordmark 18", "ink_strong", name="Craft")
        d.text(c, "Cloud", "Display/Wordmark 18", "accent_ink", name="Cloud")
        d.set(c, description="Placeholder text wordmark. Replace with the CraftCloud logo when it exists; "
                             "every header and the login page use this component.")
        # App wordmarks (two-tone), one per size; "Name" is the part before "Craft".
        x = 260
        for size in (22, 16, 14):
            c = d.comp(f"App wordmark/{size}", x, y, 10, 10, layout="horizontal", sizingH="hug", sizingV="hug")
            p = d.text(c, "Word", f"Display/Wordmark {size}", "ink_strong", name="Prefix")
            d.text(c, "Craft", f"Display/Wordmark {size}", "app:wordcraft/ink", name="Craft")
            d.prop(c, p, "Name")
            x += 200

    def buttons(self):
        d = self.d
        y = self.row(34 * 4 + 30)
        kinds = {
            "Primary": dict(fill="invert_bg", stroke=None, ink="invert_fg"),
            "Secondary": dict(fill="bg", stroke="line_strong", ink="ink"),
            "Danger": dict(fill="bad", stroke=None, ink="on_bad"),
        }
        ids = []
        for i, (kind, k) in enumerate(kinds.items()):
            for j, state in enumerate(("Default", "Hover", "Focus", "Disabled")):
                fill, stroke, ink = k["fill"], k["stroke"], k["ink"]
                if state == "Hover" and kind == "Secondary":
                    fill, stroke, ink = "invert_bg", None, "invert_fg"
                c = d.comp(f"Kind={kind}, State={state}", j * 170, y + i * 50, 120, 34, fill=fill, stroke=stroke,
                           layout="horizontal", padding=[0, 16, 0, 16], mainAlign="center", crossAlign="center",
                           sizingH="hug", sizingV="fixed")
                if state == "Hover" and kind != "Secondary":
                    d.set(c, opacity=0.8)
                if state == "Disabled":
                    d.set(c, opacity=0.5)
                if state == "Focus":
                    # A 2 px focus ring in accent ink, 2 px outside the button (an outside stroke on a
                    # transparent ring would need a wrapper; an outside stroke is what CSS outline does).
                    d.R("stroke.add", {"ids": [c], "color": "#00000000"}) if False else None
                    d.set(c, strokes=[{"type": "solid", "color": "#a9c48e"}], strokeWeight=2, strokeAlign="outside")
                    d.R("variable.bind", {"ids": [c], "field": "strokes/0/color", "variable": "focus"})
                t = d.text(c, "New", "Button/11", ink, name="Label")
                d.prop(c, t, "Label")
                ids.append(c)
        s = d.R("component.combineVariants", {"ids": ids})["id"]
        d.set(s, name="Button", x=0, y=y)
        d.C["Button"] = s
        # Square "…" button (34 × 34).
        c = d.comp("Button/More", 760, y, 34, 34, fill="bg", stroke="line_strong", layout="horizontal",
                   mainAlign="center", crossAlign="center")
        d.text(c, "…", "Body/14 Medium", "ink", name="Label")
        c = d.comp("Button/More hover", 820, y, 34, 34, fill="invert_bg", layout="horizontal",
                   mainAlign="center", crossAlign="center")
        d.text(c, "…", "Body/14 Medium", "invert_fg", name="Label")
        # Button with a trailing chevron (NEW ▾, OPEN WITH ▾).
        for kind, k in (("Secondary", kinds["Secondary"]), ("Primary", kinds["Primary"])):
            c = d.comp(f"Button/Menu {kind}", 880 if kind == "Secondary" else 1040, y, 120, 34, fill=k["fill"],
                       stroke=k["stroke"], layout="horizontal", padding=[0, 12, 0, 16], gap=10, mainAlign="center",
                       crossAlign="center", sizingH="hug", sizingV="fixed")
            t = d.text(c, "New", "Button/11", k["ink"], name="Label")
            d.prop(c, t, "Label")
            ch = d.inst("Icon/Chevron down", c, name="chevron")
            d.set(d.sub(ch, "glyph"), stroke=k["ink"])

    def badge(self):
        d = self.d
        y = self.row(20)
        ids = []
        for i, (tone, col) in enumerate((("Good", "good"), ("Warn", "warn"), ("Bad", "bad"), ("Accent", "accent_ink"),
                                         ("Muted", "muted"))):
            c = d.comp(f"Tone={tone}", i * 150, y, 60, 20, fill="sunken", stroke="line", layout="horizontal",
                       padding=[0, 6, 0, 6], mainAlign="center", crossAlign="center", sizingH="hug", sizingV="fixed")
            t = d.text(c, {"Good": "Ready", "Warn": "Experimental", "Bad": "Save failed", "Accent": "Saving…",
                           "Muted": "Read-only"}[tone], "Badge/10", col, name="Label")
            d.prop(c, t, "Label")
            ids.append(c)
        s = d.R("component.combineVariants", {"ids": ids})["id"]
        d.set(s, name="Status badge", x=0, y=y)
        d.C["Status badge"] = s

    def cells(self):
        """Toggle / filter cells (34 tall, HUD label, optional count) and the 24 px header nav cell."""
        d = self.d
        y = self.row(34)
        ids = []
        for i, state in enumerate(("Off", "On", "Hover")):
            fill, ink, cnt = {"Off": (None, "muted", "muted"), "On": ("invert_bg", "invert_fg", "invert_fg"),
                              "Hover": ("sunken", "ink_strong", "muted")}[state]
            c = d.comp(f"State={state}", i * 160, y, 60, 34, fill=fill, layout="horizontal", padding=[0, 12, 0, 12],
                       gap=8, crossAlign="center", sizingH="hug", sizingV="fixed")
            t = d.text(c, "All", "HUD/11", ink, name="Label")
            n = d.text(c, "15", "HUD/11", cnt, name="Count")
            if state == "On":
                d.set(n, opacity=0.6)
            d.prop(c, t, "Label")
            d.prop(c, n, "Count")
            d.R("component.addProperty", {"id": c, "name": "Show count", "type": "boolean", "layer": n, "default": True})
            ids.append(c)
        s = d.R("component.combineVariants", {"ids": ids})["id"]
        d.set(s, name="Cell", x=0, y=y)
        d.C["Cell"] = s
        ids = []
        for i, state in enumerate(("Off", "On")):
            fill, ink = ("invert_bg", "invert_fg") if state == "On" else (None, "muted")
            c = d.comp(f"State={state}", 600 + i * 120, y + 5, 60, 24, fill=fill, layout="horizontal",
                       padding=[0, 8, 0, 8], crossAlign="center", sizingH="hug", sizingV="fixed")
            t = d.text(c, "Apps", "HUD/11", ink, name="Label")
            d.prop(c, t, "Label")
            ids.append(c)
        s = d.R("component.combineVariants", {"ids": ids})["id"]
        d.set(s, name="Nav cell", x=600, y=y)
        d.C["Nav cell"] = s

    def inputs(self):
        d = self.d
        y = self.row(34)
        ids = []
        for i, state in enumerate(("Default", "Focus")):
            c = d.comp(f"State={state}", i * 300, y, 260, 34, fill="sunken", stroke="line", layout="horizontal",
                       padding=[0, 10, 0, 10], gap=8, crossAlign="center", sizingV="fixed")
            if state == "Focus":
                d.set(c, strokes=[{"type": "solid", "color": "#a9c48e"}], strokeWeight=2)
                d.R("variable.bind", {"ids": [c], "field": "strokes/0/color", "variable": "focus"})
            t = d.text(c, "Q3 report.docx", "Body/14", "ink", name="Value")
            d.prop(c, t, "Value")
            if state == "Focus":
                cr = d.rect(c, 0, 0, 1, 18, "ink", name="caret")
                d.set(c, gap=1)
            d.frame(c, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
            ids.append(c)
        s = d.R("component.combineVariants", {"ids": ids})["id"]
        d.set(s, name="Input", x=0, y=y)
        d.C["Input"] = s
        # Search field: icon + placeholder (muted, AA on sunken).
        c = d.comp("Search field", 640, y, 200, 34, fill="sunken", stroke="line", layout="horizontal",
                   padding=[0, 10, 0, 10], gap=8, crossAlign="center", sizingV="fixed")
        d.inst("Icon/Search", c)
        t = d.text(c, "Search apps", "Body/14", "muted", name="Placeholder")
        d.prop(c, t, "Placeholder")
        # Select (picker) with chevron.
        c = d.comp("Select", 880, y, 150, 34, fill="bg", stroke="line_strong", layout="horizontal",
                   padding=[0, 10, 0, 12], gap=8, crossAlign="center", sizingV="fixed")
        t = d.text(c, "Name", "Body/14", "ink", name="Value")
        d.set(t, sizingH="fill")
        d.prop(c, t, "Value")
        d.inst("Icon/Chevron down", c)

    def app_icon(self):
        """56 px app icon over its soft shadow (the only rounded thing: the shadow's 12 px radius)."""
        d = self.d
        y = self.row(64)
        c = d.comp("App icon", 0, y, 56, 56, fill=None, clip=False)
        sh = d.rect(c, 4, 4, 48, 48, "#00000000", name="Shadow", cornerRadius=12)
        d.effect_style(sh, "Shadow/App icon")
        d.image(c, ICON("wordcraft"), 0, 0, 56, 56, name="Icon image")
        d.set([sh], constraints={"horizontal": "scale", "vertical": "scale"})
        d.set(d.sub(c, "Icon image"), constraints={"horizontal": "scale", "vertical": "scale"})
        d.R("component.addProperty", {"id": c, "name": "Shadow", "type": "boolean", "layer": sh, "default": True})
        # Small icon (no shadow) for rows and bars.
        c = d.comp("App icon/small", 100, y, 20, 20, fill=None, clip=False)
        d.image(c, ICON("wordcraft"), 0, 0, 20, 20, name="Icon image")
        d.set(d.sub(c, "Icon image"), constraints={"horizontal": "scale", "vertical": "scale"})

    def chrome(self, W=1440):
        """Headers, footer and launcher toolbar."""
        self.header_comp("Header", ("Apps", "Files", "Recent", "Shared", "Trash"), "Apps", "Search files and apps",
                         "Craft app suite", W)
        self.header_comp("Header/Web", ("Files", "Recent", "Favourites", "Shared", "Trash", "Admin"), "Files",
                         "Search files", "Craft app suite", W)
        self.header_comp("Header/Web with apps", ("Apps", "Files", "Recent", "Favourites", "Shared", "Trash", "Admin"),
                         "Apps", "Search files and apps", "Craft app suite", W, search_w=200)
        self.footer_comp(W)

    def header_comp(self, name, nav, sel, placeholder, suite, W=1440, search_w=260):
        d = self.d
        y = self.row(48)
        h = d.comp(name, 0, y, W, 48, fill="bg", layout="horizontal", crossAlign="center", sizingV="fixed",
                   strokes=[{"type": "solid", "color": "#ffffff"}], strokeBottomWeight=1, strokeTopWeight=0,
                   strokeLeftWeight=0, strokeRightWeight=0)
        d.R("variable.bind", {"ids": [h], "field": "strokes/0/color", "variable": "line"})

        def cell(name, pad=(0, 16, 0, 16), left=False, right=False, gap=8, w=None):
            props = dict(layout="horizontal", padding=list(pad), gap=gap, crossAlign="center", sizingH="hug",
                         sizingV="fill", clip=False)
            if left or right:
                props.update(strokes=[{"type": "solid", "color": "#ffffff"}], strokeLeftWeight=1 if left else 0,
                             strokeRightWeight=1 if right else 0, strokeTopWeight=0, strokeBottomWeight=0,
                             strokeAlign="inside")
            cid = d.frame(h, name, 0, 0, w or 10, 48, **props)
            if left or right:
                d.R("variable.bind", {"ids": [cid], "field": "strokes/0/color", "variable": "line"})
            if w:
                d.set(cid, sizingH="fixed", width=w, mainAlign="center")
            return cid

        b = cell("Brand", (0, 20, 0, 20), right=True)
        d.inst("Brand/CraftCloud wordmark", b, name="Wordmark")
        s = cell("Suite", right=True)
        d.text(s, suite, "HUD/11", "muted", name="Suite label")
        n = cell("Nav", (0, 12, 0, 12), gap=4)
        for lab in nav:
            d.inst("Nav cell", n, props={"Label": lab, "State": "On" if lab == sel else "Off"}, name=f"Nav {lab}")
        d.frame(h, "Spacer", 0, 0, 10, 48, fill=None, sizingH="fill")
        sc = cell("Search", (0, 12, 0, 12), left=True)
        si = d.inst("Search field", sc, props={"Placeholder": placeholder}, name="Search")
        d.set(si, width=search_w, height=30)
        st = cell("Storage", left=True)
        d.set(st, layout="vertical", gap=6, crossAlign="min", mainAlign="center")
        d.text(st, STORAGE[0], "HUD/11", "muted", name="Storage label")
        bar = d.frame(st, "Meter", 0, 0, 150, 4, fill="line")
        d.rect(bar, 0, 0, round(150 * STORAGE[1]), 4, "accent", name="Used")
        u = cell("User", left=True, gap=10)
        av = d.frame(u, "Avatar", 0, 0, 26, 26, fill="sunken", stroke="line_strong", layout="horizontal",
                     mainAlign="center", crossAlign="center")
        d.text(av, USER[0], "Badge/10", "ink", name="Initials")
        d.text(u, USER[1], "Body/13", "ink", name="User name")
        d.inst("Icon/Chevron down", u)
        g = cell("Settings", (0, 0, 0, 0), left=True, w=48)
        d.inst("Icon/Settings", g, name="Gear")
        return h

    def footer_comp(self, W=1440):
        d = self.d
        y = self.row(34)
        f = d.comp("Footer", 0, y, W, 34, fill="bg", layout="horizontal", padding=[0, 24, 0, 24], crossAlign="center",
                   strokes=[{"type": "solid", "color": "#ffffff"}], strokeTopWeight=1, strokeBottomWeight=0,
                   strokeLeftWeight=0, strokeRightWeight=0)
        d.R("variable.bind", {"ids": [f], "field": "strokes/0/color", "variable": "line"})
        lt = d.text(f, "CraftCloud 0.1.0 · 15 apps · works offline", "HUD/11", "muted", name="Left")
        d.frame(f, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
        rt = d.text(f, "Look based on ArtCraft Launcher by the ArtCraft team", "Small/12", "muted", name="Right")
        d.prop(f, lt, "Left")
        d.prop(f, rt, "Right")

    def toolbar(self, W=1440):
        d = self.d
        y = self.row(34)
        t = d.comp("Toolbar/Launcher", 0, y, W - 48, 34, fill=None, layout="horizontal", gap=12, crossAlign="center",
                   clip=False)
        fl = d.frame(t, "Filters", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed")
        from lib import CATS
        for i, (cid, lab, n) in enumerate(CATS):
            if i:
                d.rect(fl, 0, 0, 1, 34, "line", name="sep")
            d.inst("Cell", fl, props={"Label": lab, "Count": str(n), "State": "On" if cid == "all" else "Off"},
                   name=f"Filter {lab}")
        d.frame(t, "Spacer", 0, 0, 10, 10, fill=None, sizingH="fill")
        d.text(t, "Sort", "HUD/11", "muted", name="Sort label")
        d.inst("Select", t, props={"Value": "Name"}, name="Sort")
        d.inst("Search field", t, props={"Placeholder": "Search apps"}, name="Search apps")
        tg = d.frame(t, "View toggle", 0, 0, 10, 34, stroke="line", layout="horizontal", sizingH="hug", sizingV="fixed")
        for lab, on in (("Grid", True), ("Table", False)):
            c = d.frame(tg, lab, 0, 0, 34, 34, fill="invert_bg" if on else None, layout="horizontal",
                        mainAlign="center", crossAlign="center")
            ic = d.inst(f"Icon/{'Grid' if lab == 'Grid' else 'List'}", c, name=f"{lab} icon")
            if on:
                for k in d.R("node.find", {"root": ic, "type": "RECTANGLE"})["ids"]:
                    d.set(k, fill="invert_fg")
        return t
