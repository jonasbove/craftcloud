"""Shared helpers for building the CraftCloud design in FigCraft: tokens, app data, layer helpers.

Everything goes through FigCraft's own engine commands (`figcraft-cli mcp`, see fc.py).
"""

import json
import os

from fc import FigCraft, FigCraftError

HERE = os.path.dirname(os.path.abspath(__file__))
DESIGN = os.path.dirname(HERE)
ASSETS = os.path.join(DESIGN, "assets")

# --- tokens (craft-launcher crates/ui-egui/src/theme.rs) -----------------------------------------
# alpha: ink at 60 % = 0x99, 38 % = 0x61, 15 % = 0x26, 40 % = 0x66 (dark); light 62 % = 0x9e, 42 % = 0x6b, 16 % = 0x29

THEME = {
    "bg": ("#121316", "#f2f1ee"),
    "sunken": ("#060607", "#e9e8e4"),
    "ink": ("#f2f1ee", "#101014"),
    "ink_strong": ("#ffffff", "#000000"),
    "muted": ("#f2f1ee99", "#1010149e"),
    "faint": ("#f2f1ee61", "#1010146b"),
    "line": ("#f2f1ee26", "#10101429"),
    "line_strong": ("#f2f1ee66", "#10101466"),
    "accent": ("#5e7a48", "#5e7a48"),  # CraftCloud sage (logo green), 2026-11-13
    "accent_ink": ("#a9c48e", "#3b4d2c"),
    "invert_bg": ("#ffffff", "#101014"),
    "invert_fg": ("#0b0b0c", "#f2f1ee"),
    "good": ("#3fd6b8", "#0a7563"),
    "warn": ("#ffc14d", "#8a5800"),
    "bad": ("#ff6259", "#b3261e"),
    # derived
    "on_bad": ("#0b0b0c", "#ffffff"),  # white on #ff6259 is 2.9:1; near-black is 7:1 (deviation, see README)
    "accent_tint": ("#5e7a4817", "#5e7a4812"),  # 9 % / 7 % accent wash (sheet headers, sheet.rs:31)
    "shadow": ("#0000008c", "#00000024"),  # dialogs and menus: black α140 / α36
    "scrim": ("#000000aa", "#00000069"),  # behind dialogs: α170 / α105
    "focus": ("#a9c48e", "#3b4d2c"),  # focus ring = accent_ink
    "on_accent": ("#ffffff", "#ffffff"),  # text on accent surfaces (4.8:1)
    "page": ("#0b0b0d", "#dcdbd6"),  # canvas behind the frames (not used inside the UI)
}

APPS = [
    # id, name, tagline, color, ink dark, ink light, version, card "opens", experimental, category
    ("wordcraft", "WordCraft", "Writing and document design", "#3f5fe0", "#8ea2ff", "#2a44b8", "0.18.1", [".docx", ".odt", ".md"], False, "docs"),
    ("gridcraft", "GridCraft", "Spreadsheets and data", "#1f9d55", "#4fd68a", "#12703a", "0.19.1", [".xlsx", ".ods", ".csv"], False, "docs"),
    ("deckcraft", "DeckCraft", "Presentations and slide shows", "#f07a1e", "#ffa060", "#a84f0a", "0.18.1", [".pptx", ".potx", ".ppsx"], False, "docs"),
    ("pdfcraft", "PdfCraft", "Reading, organizing and protecting PDFs", "#e8204f", "#f23a62", "#b70228", "0.21.1", [".pdf"], False, "docs"),
    ("photocraft", "PhotoCraft", "Image editing", "#2f7bf5", "#7fb0ff", "#1a5bc4", "0.23.1", [".psd", ".png", ".jpg"], True, "images"),
    ("lightcraft", "LightCraft", "Photo library and raw development", "#f2a516", "#ffc14d", "#8a5800", "0.21.1", [".cr3", ".nef", ".dng"], False, "images"),
    ("vectorcraft", "VectorCraft", "Vector illustration", "#e8573f", "#ff8a70", "#b83a24", "0.24.1", [".svg", ".ai", ".eps"], True, "images"),
    ("figcraft", "FigCraft", "Interface design and prototyping", "#02da96", "#4fe8b6", "#047a56", "0.18.1", [".figcraft", ".fig", ".sketch"], False, "images"),
    ("designcraft", "DesignCraft", "Page layout and publishing", "#7bb51c", "#a6e04a", "#457008", "0.20.1", [".idml", ".indd"], True, "images"),
    ("cadcraft", "CADCraft", "Computer-aided design and drafting", "#3ea6e8", "#7cc8ff", "#1a6fa8", "0.20.1", [".dwg", ".dxf"], False, "3d"),
    ("filmcraft", "FilmCraft", "Video editing, color and sound", "#8b5cf6", "#b69cff", "#6a3fd6", "0.20.1", [".mp4", ".mov", ".webm"], False, "av"),
    ("effectcraft", "EffectCraft", "Motion graphics and visual effects", "#e0368f", "#ff7ab8", "#b0206c", "0.24.1", [".aep", ".ecproj"], True, "av"),
    ("soundcraft", "SoundCraft", "Recording, editing and mixing audio", "#1fa5c9", "#5fd0ee", "#0e7290", "0.20.1", [".wav", ".mp3", ".flac"], True, "av"),
    ("texturecraft", "TextureCraft", "3D texture painting", "#c2672b", "#e58a4e", "#9c4a15", "0.6.1", [".glb", ".fbx", ".obj"], True, "3d"),
    ("materialcraft", "MaterialCraft", "Procedural material design", "#178a9c", "#63c5d3", "#106573", "0.4.1", [".sbs", ".mcraft"], True, "3d"),
]
APP = {a[0]: dict(id=a[0], name=a[1], prefix=a[1][:-5], tagline=a[2], color=a[3], ink_d=a[4], ink_l=a[5],
                  version=a[6], opens=a[7], exp=a[8], cat=a[9]) for a in APPS}

# What each card says under its name, and its most recent cloud file. Files are named by their format
# (DOCX, PSD), never by another vendor's product; app names come from APP, never typed again. The file
# holds them as string variables too (Apps/<id>/name, /blurb, /kind), bound to the cards and file rows.
def N(app):
    return APP[app]["name"]


BLURB = {
    "wordcraft": "Opens DOCX, ODT, RTF and Markdown files and saves them back to your cloud.",
    "gridcraft": "Opens XLSX, ODS and CSV sheets and saves them back to your cloud.",
    "deckcraft": "Opens PPTX, POTX and PPSX presentations and saves them back to your cloud.",
    "pdfcraft": "Reads, annotates and organizes PDFs; saves them back to your cloud.",
    "photocraft": "Opens PSD, layered and flat images through CraftCloud's file bridge.",
    "lightcraft": "A library: photos are added to it and exports come back. Never saves over the original.",
    "vectorcraft": "Opens SVG, AI and EPS artwork through CraftCloud's file bridge.",
    "figcraft": f"Opens {N('figcraft')}, .fig and Sketch files and saves them back to your cloud.",
    "designcraft": "Opens IDML and INDD layouts through CraftCloud's file bridge.",
    "cadcraft": "Opens DWG and DXF drawings and saves them back to your cloud.",
    "filmcraft": "Imports your clips into a project; the project saves back to your cloud.",
    "effectcraft": f"Opens AEP and {N('effectcraft')} projects through CraftCloud's file bridge.",
    "soundcraft": "Imports audio into a session through CraftCloud's file bridge.",
    "texturecraft": "Opens GLB, FBX and OBJ models through CraftCloud's file bridge.",
    "materialcraft": f"Opens SBS and {N('materialcraft')} graphs through CraftCloud's file bridge.",
}
# The type of each app's typical file, by its format (the app that opens it is said beside it).
KIND = {
    "wordcraft": "DOCX document",
    "gridcraft": "XLSX spreadsheet",
    "deckcraft": "PPTX presentation",
    "photocraft": "PSD image",
}
RECENT = {
    "wordcraft": "Recent: Q3 report.docx · 2 h ago",
    "gridcraft": "Recent: Budget 2027.xlsx · yesterday",
    "deckcraft": "Recent: Pitch.pptx · 3 days ago",
    "pdfcraft": "Recent: Contract.pdf · 5 h ago",
    "photocraft": "Recent: Poster.psd · 20 min ago",
    "lightcraft": "Recent: IMG_2041.CR3 · Monday",
    "vectorcraft": "Recent: Logo.svg · 1 h ago",
    "figcraft": "Recent: CraftCloud UI.figcraft · just now",
    "designcraft": "No recent files",
    "cadcraft": "Recent: Floorplan.dwg · 2 days ago",
    "filmcraft": "Recent: Trailer.mp4 · last week",
    "effectcraft": "No recent files",
    "soundcraft": "Recent: Theme.wav · 4 days ago",
    "texturecraft": "No recent files",
    "materialcraft": "No recent files",
}
CATS = [("all", "All", 15), ("docs", "Documents", 4), ("images", "Images & Design", 5), ("av", "Video & Audio", 3), ("3d", "3D", 3)]
ORDER = ["wordcraft", "gridcraft", "photocraft",  # pinned
         "deckcraft", "pdfcraft", "lightcraft", "vectorcraft", "figcraft", "designcraft", "cadcraft",
         "filmcraft", "effectcraft", "soundcraft", "texturecraft", "materialcraft"]
PINNED = 3

USER = ("JD", "John Doe")  # demo account
STORAGE = ("12.4 GB of 100 GB", 0.124)
VERSION = "0.2.0"
OXICLOUD = "0.10.0"

# --- text styles ---------------------------------------------------------------------------------

DISPLAY = {"fontFamily": "Archivo", "fontVariations": {"wght": 620, "wdth": 118}}
TEXT_STYLES = {
    "Display/Page title 32": dict(DISPLAY, fontSize=32, letterSpacing="-2%"),
    "Display/Sheet title 26": dict(DISPLAY, fontSize=26, letterSpacing="-2%"),
    "Display/Heading 24": dict(DISPLAY, fontSize=24, letterSpacing="-2%"),
    "Display/Dialog title 22": dict(DISPLAY, fontSize=22, letterSpacing="-2%"),
    "Display/Wordmark 22": dict(DISPLAY, fontSize=22, letterSpacing="-3%"),
    "Display/Wordmark 18": dict(DISPLAY, fontSize=18, letterSpacing="-3%"),
    "Display/Wordmark 16": dict(DISPLAY, fontSize=16, letterSpacing="-3%"),
    "Display/Wordmark 14": dict(DISPLAY, fontSize=14, letterSpacing="-3%"),
    "Display/Wordmark 32": dict(DISPLAY, fontSize=32, letterSpacing="-3%"),
    "Body/14": {"fontFamily": "Inter", "fontSize": 14, "fontVariations": {"wght": 400, "opsz": 14}, "lineHeight": 20},
    "Body/14 Medium": {"fontFamily": "Inter", "fontSize": 14, "fontVariations": {"wght": 500, "opsz": 14}, "lineHeight": 20},
    "Body/13.5 Tagline": {"fontFamily": "Inter", "fontSize": 13.5, "fontVariations": {"wght": 400, "opsz": 14}, "lineHeight": 18},
    "Body/13": {"fontFamily": "Inter", "fontSize": 13, "fontVariations": {"wght": 400, "opsz": 14}, "lineHeight": 18},
    "Body/13 Medium": {"fontFamily": "Inter", "fontSize": 13, "fontVariations": {"wght": 500, "opsz": 14}, "lineHeight": 18},
    "Small/12.5": {"fontFamily": "Inter", "fontSize": 12.5, "fontVariations": {"wght": 400, "opsz": 14}, "lineHeight": 18},
    "Small/12": {"fontFamily": "Inter", "fontSize": 12, "fontVariations": {"wght": 400, "opsz": 14}, "lineHeight": 16},
    "HUD/11": {"fontFamily": "Geist Mono", "fontSize": 11, "fontVariations": {"wght": 500}, "letterSpacing": "15%", "textCase": "upper"},
    "Button/11": {"fontFamily": "Geist Mono", "fontSize": 11, "fontVariations": {"wght": 700}, "letterSpacing": "12%", "textCase": "upper"},
    "Badge/10": {"fontFamily": "Geist Mono", "fontSize": 10, "fontVariations": {"wght": 500}, "letterSpacing": "10%", "textCase": "upper"},
    "Mono/13": {"fontFamily": "Geist Mono", "fontSize": 13, "fontVariations": {"wght": 400}},
    "Display/Hero 56": dict(DISPLAY, fontSize=56, letterSpacing="-2%", lineHeight=60),
    "Display/Hero 40": dict(DISPLAY, fontSize=40, letterSpacing="-2%", lineHeight=44),
    "Body/18": {"fontFamily": "Inter", "fontSize": 18, "fontVariations": {"wght": 400, "opsz": 14}, "lineHeight": 28},
    "Body/16 Medium": {"fontFamily": "Inter", "fontSize": 16, "fontVariations": {"wght": 500, "opsz": 14}, "lineHeight": 20},
    "Mono/28 Code": {"fontFamily": "Geist Mono", "fontSize": 28, "fontVariations": {"wght": 500}, "letterSpacing": "8%"},
    "Mono/12": {"fontFamily": "Geist Mono", "fontSize": 12, "fontVariations": {"wght": 400}},
}


def paint(c):
    """'ink' -> Theme variable, 'app:wordcraft/ink' -> Apps variable, '#hex' -> literal, None -> none."""
    if c is None:
        return "none"
    if c.startswith("#"):
        return c
    if c.startswith("app:"):
        return {"$var": "Apps/" + c[4:]}
    return {"$var": "Theme/" + c}


class Doc:
    def __init__(self, cli, fonts=True):
        self.fc = FigCraft(cli, {"FIGCRAFT_NO_SYSTEM_FONTS": "1"})
        self.R = self.fc.run
        if fonts:
            for f in ("Inter-Variable.ttf", "Archivo-Variable.ttf", "GeistMono-Variable.ttf"):
                self.R("text.loadFont", {"path": os.path.join(ASSETS, "fonts", f)})
        self.C = {}  # component name -> id

    # --- setup ---------------------------------------------------------------------------------
    def tokens(self):
        R = self.R
        R("variable.createCollection", {"name": "Theme", "modes": ["Dark", "Light"]})
        for k, (d, l) in THEME.items():
            R("variable.create", {"collection": "Theme", "name": k, "type": "color", "values": {"Dark": d, "Light": l}})
        R("variable.createCollection", {"name": "Apps", "modes": ["Dark", "Light"]})
        for a in APP.values():
            i = a["id"]
            R("variable.create", {"collection": "Apps", "name": f"{i}/color", "type": "color", "values": {"Dark": a["color"], "Light": a["color"]}})
            R("variable.create", {"collection": "Apps", "name": f"{i}/ink", "type": "color", "values": {"Dark": a["ink_d"], "Light": a["ink_l"]}})
            R("variable.create", {"collection": "Apps", "name": f"{i}/wash", "type": "color", "values": {"Dark": a["color"] + "0d", "Light": a["color"] + "0d"}})
            for key, val in (("name", a["name"]), ("blurb", BLURB.get(i)), ("kind", KIND.get(i))):
                if val:
                    R("variable.create", {"collection": "Apps", "name": f"{i}/{key}", "type": "string", "values": {"Dark": val, "Light": val}})
        # Text styles are made from specimen layers: `style.create {value}` drops fontVariations and
        # textCase (FigCraft 0.18.1), `style.create {from}` keeps them.
        y = 0
        spec = self.frame(None, "Type specimens", 0, 0, 760, 40 * len(TEXT_STYLES) + 40, fill="bg")
        for name, v in TEXT_STYLES.items():
            label = name.split("/", 1)[1]
            tid = self.node("text", spec, name, 24, 24 + y, text=f"{name}  ·  Craft app suite 0.18.1", fill="ink", **v)
            R("style.create", {"type": "text", "name": name, "from": tid})
            R("style.apply", {"ids": [tid], "style": name})
            y += 40
        self.specimens = spec
        # Effect styles; colour bound to the theme's shadow variable so dark and light differ.
        R("style.create", {"type": "effect", "name": "Shadow/Dialog", "value": [
            {"type": "dropShadow", "color": "#0000008c", "offset": [0, 8], "radius": 24}]})
        try:
            R("style.bind", {"style": "Shadow/Dialog", "field": "effects/0/color", "variable": "shadow"})
        except FigCraftError as e:
            print("note: style.bind effect color failed:", e)
        R("style.create", {"type": "effect", "name": "Shadow/App icon", "value": [
            {"type": "dropShadow", "color": "#00000047", "offset": [0, 6], "radius": 14}]})

    def light(self, ids):
        for col in ("Theme", "Apps"):
            self.R("variable.setMode", {"ids": ids, "collection": col, "mode": "Light"})

    def dark(self, ids):
        for col in ("Theme", "Apps"):
            self.R("variable.setMode", {"ids": ids, "collection": col, "mode": "Dark"})

    def page(self, name, first=False):
        if first:
            pid = self.R("page.list")
            pages = pid.get("pages", pid) if isinstance(pid, dict) else pid
            p0 = pages[0]["id"]
            self.R("page.rename", {"id": p0, "name": name})
            self.R("page.select", {"id": p0})
            self.current_page = p0
            return p0
        r = self.R("page.add", {"name": name})
        self.R("page.select", {"id": r["id"]})
        self.current_page = r["id"]
        return r["id"]

    # --- layers --------------------------------------------------------------------------------
    def node(self, typ, parent=None, name=None, x=0, y=0, w=None, h=None, **props):
        p = {"type": typ, "x": x, "y": y, "select": False}
        if parent is not None:
            p["parent"] = parent
        if name:
            p["name"] = name
        if w is not None:
            p["width"] = w
        if h is not None:
            p["height"] = h
        for k in ("text", "path", "closed", "points"):
            if k in props:
                p[k] = props.pop(k)
        for k in ("fill", "stroke"):
            if k in props:
                props[k] = paint(props[k])
        if props:
            p["props"] = props
        return self.R("node.create", p)["id"]

    def frame(self, parent, name, x, y, w, h, fill=None, stroke=None, clip=True, **props):
        props.setdefault("fill", fill)
        if stroke:
            props["stroke"] = stroke
            props.setdefault("strokeWeight", 1)
            props.setdefault("strokeAlign", "inside")
        props.setdefault("clip", clip)
        return self.node("frame", parent, name, x, y, w, h, **props)

    def comp(self, name, x, y, w, h, fill=None, stroke=None, **props):
        props.setdefault("fill", fill)
        if stroke:
            props["stroke"] = stroke
            props.setdefault("strokeWeight", 1)
            props.setdefault("strokeAlign", "inside")
        cid = self.node("component", None, name, x, y, w, h, **props)
        self.C[name] = cid
        return cid

    def rect(self, parent, x, y, w, h, fill, name=None, **props):
        return self.node("rectangle", parent, name or "rect", x, y, w, h, fill=fill, **props)

    def hline(self, parent, x, y, w, color="line", name="hairline"):
        return self.rect(parent, x, y, w, 1, color, name=name)

    def vline(self, parent, x, y, h, color="line", name="hairline"):
        return self.rect(parent, x, y, 1, h, color, name=name)

    def text(self, parent, chars, style, color="ink", x=0, y=0, name=None, w=None, align=None, **props):
        props["textStyle"] = style
        props["fill"] = color
        if w is not None:
            props.setdefault("textAutoResize", "height")
        if align:
            props["textAlign"] = align
        return self.node("text", parent, name or chars[:40], x, y, w, None, text=chars, **props)

    def image(self, parent, path, x, y, w, h, name="image", mode="fill"):
        return self.R("image.place", {"path": path, "parent": parent, "x": x, "y": y, "width": w, "height": h,
                                      "name": name, "mode": mode})["id"]

    def set(self, ids, **props):
        if not isinstance(ids, list):
            ids = [ids]
        for k in ("fill", "stroke"):
            if k in props:
                props[k] = paint(props[k])
        return self.R("node.set", {"ids": ids, "props": props})

    def absolute(self, nid, x, y, w=None, h=None):
        """Take a layer out of its auto layout flow and pin it at x, y (FigCraft keeps the flow position
        when `absolute` is set, so the position has to follow in a second call)."""
        self.set(nid, absolute=True)
        p = {"x": x, "y": y}
        if w is not None:
            p["width"] = w
        if h is not None:
            p["height"] = h
        self.set(nid, **p)

    def effect_style(self, ids, style):
        if not isinstance(ids, list):
            ids = [ids]
        self.R("style.apply", {"ids": ids, "style": style})

    # --- components ----------------------------------------------------------------------------
    def inst(self, comp, parent=None, x=0, y=0, props=None, w=None, h=None, name=None):
        cid = self.C[comp] if isinstance(comp, str) else comp
        p = {"component": cid, "x": x, "y": y}
        if parent is not None:
            p["parent"] = parent
        iid = self.R("instance.create", p)["id"]
        if props:
            self.R("instance.setProperties", {"ids": [iid], "properties": props})
        sz = {}
        if w is not None:
            sz["width"] = w
        if h is not None:
            sz["height"] = h
        if name:
            sz["name"] = name
        if sz:
            self.set(iid, **sz)
        return iid

    def find(self, root, name):
        """Exact-name descendants of `root`."""
        r = self.R("node.find", {"root": root, "name": name, "limit": 200})
        out = []
        for i in r["ids"]:
            if self.R("node.get", {"id": i, "fields": ["name"]})["name"] == name:
                out.append(i)
        return out

    def sub(self, root, name):
        ids = self.find(root, name)
        if not ids:
            raise FigCraftError(f"no layer {name!r} under {root}")
        return ids[0]

    def prop(self, comp_id, layer_id, name, typ="text"):
        self.R("component.addProperty", {"id": comp_id, "name": name, "type": typ, "layer": layer_id})

    def export(self, ids, path, scale=1):
        return self.fc.tool("export", {"ids": ids if isinstance(ids, list) else [ids], "format": "png",
                                       "scale": scale, "path": path})

    def save(self, path):
        return self.fc.tool("save_file", {"path": path})
