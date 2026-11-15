"""v2 · Sharing (issue #17): the share-link frames.

- NEW in 0.2.0 (issue #18): the "Create share link" sheet, opened from a file's context menu
  ("Share link…") or its details panel ("Share"): the item, expiry, an optional password, then the
  link with Copy; the date picker, the created state, the copied toast and the error state.
- NEW in 0.2.0 (issue #19): "Your share links", opened from the account menu under the person's
  name in the header (and from the created sheet): the list, Copy, Revoke with a confirmation, and the
  empty state.
- 0.1.0 as it is (crates/ui-egui/src/web/share.rs): the public page a link opens (`/s/<token>`), a
  file, a folder and a subfolder, the password prompt and its wrong-password state, the expired and
  removed link, and the shared file on a phone.

Every frame is built from v2.py's helpers and components; the few icons this area needs that v2 has
not got (copy, eye, link, calendar) are made here, on this page, as `v2/Icon/<name>` components.
"""

from lib import KIND, VERSION
from screens import hud, vstack, hstack, side_stroke
import v2
from v2 import (W, H, GAP, HOST, OWNER, page, sheet, sheet_actions, ibtn, btn, cells, spacer, panel, note,
                scrim, toast, square, toolbar, headline, meta)

PAGE = "v2 · Sharing"
PHONE_W, PHONE_H = 390, 844
TOKEN = "aeddf06a-4200-4b05-ad1c-272cf305e350"  # the shape OxiCloud gives (share-create.json)
LINK = f"https://{HOST}/s/{TOKEN}"

ICONS = {
    "copy": "M4.5 4.5 L12.5 4.5 L12.5 12.5 L4.5 12.5 Z M9.5 4.5 L9.5 1.5 L1.5 1.5 L1.5 9.5 L4.5 9.5",
    "eye": "M1 7 C3 3.5 5 2.5 7 2.5 C9 2.5 11 3.5 13 7 C11 10.5 9 11.5 7 11.5 C5 11.5 3 10.5 1 7 Z "
           "M5.2 7 C5.2 4.6 8.8 4.6 8.8 7 C8.8 9.4 5.2 9.4 5.2 7 Z",
    "eye-off": "M1 7 C3 3.5 5 2.5 7 2.5 C9 2.5 11 3.5 13 7 C11 10.5 9 11.5 7 11.5 C5 11.5 3 10.5 1 7 Z "
               "M2 12 L12 2",
    "link": "M6 8 C7 9.2 8.6 9.2 9.6 8.2 L11.8 6 C12.8 5 12.8 3.4 11.8 2.4 C10.8 1.4 9.2 1.4 8.2 2.4 L7.4 3.2 "
            "M8 6 C7 4.8 5.4 4.8 4.4 5.8 L2.2 8 C1.2 9 1.2 10.6 2.2 11.6 C3.2 12.6 4.8 12.6 5.8 11.6 L6.6 10.8",
    "calendar": "M1.5 3 L12.5 3 L12.5 12.5 L1.5 12.5 Z M1.5 6 L12.5 6 M4.5 1.5 L4.5 4 M9.5 1.5 L9.5 4",
    "chevron-left": "M9 2 L4 7 L9 12",
    "chevron-right": "M5 2 L10 7 L5 12",
}

# The files behind the create sheet (My files / Documents / Reports).
BEHIND = [("Q3 report.docx", "wordcraft", "Today 11:42", "248 KB"),
          ("Q2 report.docx", "wordcraft", "4 Aug 2026", "231 KB"),
          ("Budget 2027.xlsx", "gridcraft", "Yesterday 16:05", "86 KB"),
          ("Board pitch.pptx", "deckcraft", "10 Nov 2026", "5.8 MB")]

EXPIRY = ("Never", "1 day", "7 days", "30 days", "On a date…")

# Your share links: (name, app or "folder", path now or None when gone, expires, state, password, opened, made)
LINKS = [
    ("Q3 report.docx", "wordcraft", "My files / Documents / Reports", "21 Nov 2026", "in 7 days", True, 3,
     "Today 14:32"),
    ("Team", "folder", "My files / Projects", "Never", None, False, 12, "10 Nov 2026"),
    ("Board pitch.pptx", "deckcraft", "My files / Clients / North", "14 Dec 2026", "in 30 days", True, 0,
     "Today 09:05"),
    ("Poster.psd", "photocraft", None, "Never", None, False, 7, "2 Nov 2026"),
    ("Budget 2026.xlsx", "gridcraft", "My files / Finance", "2 Nov 2026", "expired", False, 5, "26 Oct 2026"),
]


# --- parts made here ------------------------------------------------------------------------------------

def parts(d):
    """The icons this page adds, as v2/Icon/<name> components (14 px, 1.5 px strokes) above the frames."""
    for i, (n, path) in enumerate(ICONS.items()):
        c = d.comp(f"v2/Icon/{n}", i * 40, -200, 14, 14, fill=None, clip=False)
        d.node("vector", c, "glyph", 0, 0, path=path, closed=False, fill=None, stroke="ink", strokeWeight=1.5)
    hud(d, None, "Sharing icons (v2/Icon/…)", 0, -230, color="muted", name="Parts label")


def new_tag(d, parent, name="New in 0.2.0"):
    """The "New in 0.2.0" HUD tag: 20 tall, 1 px accent border, Badge 10 in accent_ink."""
    t = d.frame(parent, name, 0, 0, 10, 20, fill=None, stroke="accent", layout="horizontal", padding=[0, 6, 0, 6],
                crossAlign="center", sizingH="hug", sizingV="fixed", clip=False)
    d.text(t, "New in 0.2.0", "Badge/10", "accent_ink", name="Tag label")
    return t


def fix_header(d, f):
    """The page's header (the shared v2 header shows Files as available, as in 0.1.0)."""
    return d.sub(f, "Header")


def icon_for(d, parent, app, size=20, name="Item icon"):
    if app == "folder":
        return d.inst("Icon/Folder", parent, name=name)
    ic = d.inst("Icon/File", parent, name=name)
    d.set(d.sub(ic, "page"), fill=f"app:{app}/color")
    return ic


def area_center_y(h, sh):
    """chrome::sheet centres the sheet in the area between the header (48) and the footer (34)."""
    return 48 + (h - 48 - 34 - sh) / 2


def files_behind(d, f):
    """A plain Files page under the scrim: the crumbs and four rows (the Files frames are the spec for it)."""
    tb = toolbar(d, f, 24, 72, W - 48)
    square(d, tb, "refresh", "Refresh")
    hud(d, tb, "My files / Documents / Reports", color="ink", name="Crumbs")
    spacer(d, tb)
    d.inst("Search field", tb, props={"Placeholder": "Search files"}, name="Search")
    t = d.frame(f, "Files table", 24, 130, W - 48, 10, fill="bg", stroke="line", layout="vertical",
                padding=[0, 16, 0, 16], sizingV="hug", clip=False)
    for i, (nm, app, when, size) in enumerate(BEHIND):
        r = d.frame(t, f"Row {nm}", 0, 0, W - 80, 48, fill="sunken" if i == 0 else None, layout="horizontal", gap=10,
                    crossAlign="center", sizingH="fill")
        if i < len(BEHIND) - 1:
            side_stroke(d, r, bottom=1)
        icon_for(d, r, app)
        n = d.text(r, nm, "Body/14", "ink", name="Name")
        d.set(n, sizingH="fill")
        d.text(r, when, "Small/12", "muted", name="Modified")
        s = d.text(r, size, "Mono/12", "muted", name="Size", w=80, align="right")
        del s


# --- NEW: create share link -------------------------------------------------------------------------------

def item_box(d, parent, w, name="Q3 report.docx", app="wordcraft", facts=None):
    """The item the link shares: icon, name, and a line with its type, size and folder."""
    b = d.frame(parent, "Item", 0, 0, w, 10, fill="sunken", stroke="line", layout="horizontal", gap=12,
                padding=[10, 12, 10, 12], crossAlign="center", sizingH="fill", sizingV="hug", clip=False)
    icon_for(d, b, app)
    col = vstack(d, b, "Item text", 0, 0, w - 60, gap=2)
    d.set(col, sizingH="fill")
    d.text(col, name, "Body/14 Medium", "ink_strong", name="Item name")
    d.text(col, facts or f"{KIND[app]} · 248 KB · in My files / Documents / Reports", "Small/12", "muted",
           name="Item facts")
    return b


def label(d, parent, text, extra=None):
    r = hstack(d, parent, f"Label {text}", gap=8)
    hud(d, r, text, color="ink", name="Label")
    if extra:
        hud(d, r, extra, color="muted", name="Label extra")
    return r


def expiry_block(d, body, w, sel="7 days", date=None):
    fl = vstack(d, body, "Field Expires", 0, 0, w, gap=6)
    d.set(fl, sizingH="fill")
    label(d, fl, "Link expires")
    cells(d, fl, EXPIRY, sel=sel, name="Expiry")
    if date:
        r = hstack(d, fl, "Date row", gap=8)
        i = d.inst("Input", r, props={"State": "Focus", "Value": date}, name="Date input")
        d.set(i, sizingH="fixed", width=200)
        sq = square(d, r, "calendar", "Calendar button")
        d.set(sq, fill="invert_bg")
        d.set(d.sub(sq, "glyph"), stroke="invert_fg")
        note(d, fl, f"The link stops working at the end of {date}.", w, name="Expiry hint")
    elif sel == "Never":
        note(d, fl, "The link works until you revoke it.", w, name="Expiry hint")
    else:
        note(d, fl, "The link stops working on 21 Nov 2026 at 14:32.", w, name="Expiry hint")
    return fl


def password_block(d, body, w, value="••••••••••", shown=False, placeholder=False):
    fl = vstack(d, body, "Field Password", 0, 0, w, gap=6)
    d.set(fl, sizingH="fill")
    label(d, fl, "Password", "optional")
    r = hstack(d, fl, "Password row", gap=8)
    d.set(r, sizingH="fill")
    i = d.inst("Input", r, props={"Value": value}, name="Password input")
    d.set(i, sizingH="fill")
    if placeholder:
        d.set(d.sub(i, "Value"), fill="muted")
    ibtn(d, r, "Hide" if shown else "Show", "eye-off" if shown else "eye", "Secondary", name="Show password")
    note(d, fl, "The person opening the link will need it. Send it to them separately, not in the same message "
                "as the link.", w, name="Password hint")
    return fl


def create_frame(d, name, x, y, light=False, state="form"):
    """state: form · date (the date picker open, password shown) · error · created · copied."""
    f = page(d, name, x, y, light=light, nav="Files")
    fix_header(d, f)
    files_behind(d, f)
    scrim(d, f, W, H)
    sw = 560
    created = state in ("created", "copied")
    sh = {"form": 482, "date": 522, "error": 556, "created": 452, "copied": 452}[state]
    title, caption = (("Link ready", "Q3 report.docx · anyone with the link can open it") if created else
                      ("Share link", "Q3 report.docx · anyone with the link can open it"))
    s, body = sheet(d, f, (W - sw) / 2, area_center_y(H, sh), sw, sh, title, caption,
                    right=lambda hd: new_tag(d, hd))
    bw = sw - 48
    if not created:
        item_box(d, body, bw)
        expiry_block(d, body, bw, sel="On a date…" if state == "date" else "7 days",
                     date="28 Nov 2026" if state == "date" else None)
        # The error frame also shows the password after SHOW (the date picker would cover it).
        password_block(d, body, bw, value="maple-harbour-41" if state == "error" else "••••••••••••••••",
                       shown=state == "error")
        if state == "error":
            e = d.frame(body, "Error", 0, 0, bw, 10, fill="sunken", stroke="line", layout="horizontal", gap=10,
                        padding=[10, 12, 10, 12], crossAlign="min", sizingH="fill", sizingV="hug", clip=False)
            d.inst("Status badge", e, props={"Tone": "Bad", "Label": "Not made"}, name="Error badge")
            t = d.text(e, "The server didn't make the link: you don't have permission to share this file. Nothing "
                          "was shared. Try again, or ask your admin.", "Small/12.5", "bad", name="Error text",
                       w=bw - 120)
            d.set(t, sizingH="fill")
        spacer(d, body, v=True)
        sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"),
                      right=lambda r: ibtn(d, r, "Try again" if state == "error" else "Create link",
                                           "refresh" if state == "error" else "link"))
    else:
        fl = vstack(d, body, "Field Link", 0, 0, bw, gap=6)
        d.set(fl, sizingH="fill")
        label(d, fl, "Link")
        r = hstack(d, fl, "Link row", gap=8)
        d.set(r, sizingH="fill")
        i = d.inst("Input", r, props={"Value": LINK}, name="Link field")
        d.set(i, sizingH="fill")
        v = d.sub(i, "Value")
        d.set(v, sizingH="fill", textAutoResize="truncate", maxLines=1)
        d.set(v, textStyle="Mono/12")
        d.set(d.sub(i, "Spacer"), visible=False)
        if state == "copied":
            b = ibtn(d, r, "Copied", "check", "Primary", name="Copy")
            d.set(d.sub(d.sub(b, "Icon"), "glyph"), stroke="invert_fg")
        else:
            ibtn(d, r, "Copy", "copy", "Primary", name="Copy")
        note(d, fl, "Read-only. Anyone with this link can open the file without signing in.", bw, name="Link hint")
        p = panel(d, body, "This link", bw)
        for k, val in (("Shares", "My files / Documents / Reports / Q3 report.docx"),
                       ("Expires", "21 Nov 2026, 14:32 · in 7 days"),
                       ("Password", "On · it isn't shown again")):
            rr = hstack(d, p, k, gap=0)
            d.set(rr, sizingH="fill", crossAlign="min")
            kk = hud(d, rr, k, color="muted", name="k")
            d.set(kk, sizingH="fixed", width=110)
            vv = d.text(rr, val, "Body/14", "ink_strong", name="v", w=bw - 28 - 110)
            d.set(vv, sizingH="fill")
        lr = hstack(d, p, "Lock row", gap=8)
        d.inst("Icon/Lock", lr, name="Lock")
        d.text(lr, "Send the password separately.", "Small/12", "muted", name="Lock note")
        spacer(d, body, v=True)

        def left(r):
            t = d.text(r, "See all your share links", "Small/12.5", "accent_ink", name="All links")
            d.set(t, textDecoration="underline")
        sheet_actions(d, s, sw, left=left, right=lambda r: ibtn(d, r, "Done", "check", "Secondary"))
        if state == "copied":
            toast(d, f, W - 24 - 420, H - 34 - 24 - 70, 420, "good", "Link copied",
                  "Paste it into a message to the people you share with.")
    if state == "date":
        date_picker(d, f, s)
    return f


def date_picker(d, f, s):
    """A month grid under the date field: Monday first, past days muted, today boxed, the pick inverted."""
    di = d.sub(s, "Date input")
    b = d.R("node.get", {"id": di, "fields": ["absoluteBounds"]})["absoluteBounds"]
    fb = d.R("node.get", {"id": f, "fields": ["absoluteBounds"]})["absoluteBounds"]
    cw, chh = 34, 30
    pw = 7 * cw + 24
    p = d.frame(f, "Date picker", b["x"] - fb["x"], b["y"] - fb["y"] + 34 + 4, pw, 10, fill="bg", stroke="line",
                layout="vertical", gap=6, padding=[10, 12, 12, 12], sizingV="hug", clip=False)
    d.effect_style(p, "Shadow/Dialog")
    hd = hstack(d, p, "Month row", gap=8)
    d.set(hd, sizingH="fill")
    for nm, ic in (("Previous month", "chevron-left"),):
        q = square(d, hd, ic, nm)
        d.set(q, width=28, height=28)
    m = d.text(hd, "November 2026", "Body/14 Medium", "ink_strong", name="Month", align="center")
    d.set(m, sizingH="fill")
    q = square(d, hd, "chevron-right", "Next month")
    d.set(q, width=28, height=28)
    wk = hstack(d, p, "Weekdays", gap=0)
    for dn in ("Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"):
        c = d.frame(wk, dn, 0, 0, cw, 20, fill=None, layout="horizontal", mainAlign="center", crossAlign="center")
        d.text(c, dn, "Badge/10", "muted", name="d")
    days = [None] * 6 + list(range(1, 31))
    for row_i in range((len(days) + 6) // 7):
        rr = hstack(d, p, f"Week {row_i + 1}", gap=0)
        for dd in days[row_i * 7:row_i * 7 + 7]:
            sel, today, past = dd == 28, dd == 14, dd is not None and dd < 14
            c = d.frame(rr, f"Day {dd or '-'}", 0, 0, cw, chh, fill="invert_bg" if sel else None,
                        stroke="line_strong" if today else None, layout="horizontal", mainAlign="center",
                        crossAlign="center")
            if dd:
                d.text(c, str(dd), "Mono/12", "invert_fg" if sel else ("muted" if past else "ink"), name="n")
    note(d, p, "Past days can't be picked.", pw - 24, name="Picker note")
    return p


# --- NEW: your share links --------------------------------------------------------------------------------

LCOLS = [("Shared item", None), ("Expires", 170), ("Password", 110), ("Opened", 100), ("Made", 120), ("", 230)]


def links_toolbar(d, f, empty=False):
    tb = toolbar(d, f, 24, 72, W - 48)
    square(d, tb, "refresh", "Refresh")
    if empty:
        cells(d, tb, ("All", "Active", "Expired"), sel="All", name="Filter", counts=(0, 0, 0))
    else:
        cells(d, tb, ("All", "Active", "Expired", "No longer there"), sel="All", name="Filter", counts=(5, 3, 1, 1))
    new_tag(d, tb)
    spacer(d, tb)
    d.inst("Search field", tb, props={"Placeholder": "Search links"}, name="Search")
    hud(d, tb, "Sort", color="muted", name="Sort caption")
    d.inst("Select", tb, props={"Value": "Newest"}, name="Sort")
    return tb


def links_table(d, f, y, hover=None):
    tw = W - 48
    t = d.frame(f, "Links table", 24, y, tw, 10, fill="bg", stroke="line", layout="vertical", padding=[0, 16, 0, 16],
                sizingV="hug", clip=False)
    hd = d.frame(t, "Header row", 0, 0, tw - 32, 42, fill=None, layout="horizontal", crossAlign="center",
                 sizingH="fill")
    side_stroke(d, hd, bottom=1)
    for lab, cw in LCOLS:
        c = d.frame(hd, f"H {lab or 'actions'}", 0, 0, cw or 100, 42, fill=None, layout="horizontal", gap=6,
                    crossAlign="center", sizingH="fill" if cw is None else "fixed", sizingV="fill")
        if lab:
            hud(d, c, lab, color="ink" if lab == "Made" else "muted", name="Label")
        if lab == "Made":
            d.inst("Icon/Sort up", c, name="Sort arrow")
    for i, (nm, app, path, exp, when, pw, opened, made) in enumerate(LINKS):
        gone, expired = path is None, when == "expired"
        r = d.frame(t, f"Row {nm}", 0, 0, tw - 32, 62, fill="sunken" if nm == hover else None, layout="horizontal",
                    crossAlign="center", sizingH="fill")
        if i < len(LINKS) - 1:
            side_stroke(d, r, bottom=1)

        def c(cw, nmc):
            return d.frame(r, nmc, 0, 0, cw or 100, 62, fill=None, layout="horizontal", gap=10, crossAlign="center",
                           sizingH="fill" if cw is None else "fixed", sizingV="fill", clip=True)
        a = c(None, "Item")
        ic = icon_for(d, a, app)
        if gone:
            d.set(ic, opacity=0.5)
        col = vstack(d, a, "Names", 0, 0, 400, gap=3)
        d.text(col, nm, "Body/16 Medium", "muted" if gone else "ink_strong", name="Name")
        if gone:
            gr = hstack(d, col, "Gone", gap=8)
            d.inst("Status badge", gr, props={"Tone": "Warn", "Label": "No longer there"}, name="Gone badge")
            d.text(gr, f"Shared as {nm} · moved to the trash or deleted", "Small/12", "muted", name="Shared as")
        else:
            d.text(col, path + (" /" if app == "folder" else ""), "Small/12", "muted", name="Where now")
        b = c(170, "Expires")
        d.set(b, layout="vertical", gap=4, mainAlign="center", crossAlign="min")
        if expired:
            d.inst("Status badge", b, props={"Tone": "Bad", "Label": "Expired"}, name="Expired")
            d.text(b, exp, "Small/12", "muted", name="Date")
        elif when:
            d.text(b, exp, "Body/14", "ink", name="Date")
            d.text(b, when, "Small/12", "muted", name="When")
        else:
            d.text(b, "Never", "Body/14", "muted", name="Date")
        p_ = c(110, "Password")
        if pw:
            d.inst("Icon/Lock", p_, name="Lock")
            d.text(p_, "Yes", "Body/14", "ink", name="Has password")
        else:
            d.text(p_, "No", "Body/14", "muted", name="Has password")
        o = c(100, "Opened")
        d.text(o, "Not yet" if not opened else f"{opened} time{'s' if opened != 1 else ''}", "Mono/12",
               "muted", name="Opened")
        m_ = c(120, "Made")
        d.text(m_, made, "Small/12", "muted", name="Made")
        ac = c(230, "Actions")
        d.set(ac, mainAlign="max", gap=8)
        ibtn(d, ac, "Copy", "copy", "Secondary", name="Copy",
             state="Disabled" if (gone or expired) else None)
        btn(d, ac, "Revoke")
    return t


def links_frame(d, name, x, y, mode="list", light=False):
    """mode: list (Copy just clicked: the toast) · revoke (the confirmation) · empty."""
    f = page(d, name, x, y, light=light, nav="")
    fix_header(d, f)
    d.set(d.sub(d.sub(f, "Header"), "User"), fill="sunken")
    if mode == "empty":
        links_toolbar(d, f, empty=True)
        top = vstack(d, f, "Empty", 24, 160, 760, gap=16)
        hud(d, top, "Your share links", color="muted", name="Kicker")
        headline(d, top, "No share *links*\nyet.", style="Display/Hero 40", light=light)
        d.text(top, "Make one from Files: open a file's or folder's menu and choose Share link…, or press Share in "
                    "its details. Every link you make shows here, with where the item is now and when the link "
                    "expires.", "Body/18", "muted", name="Body", w=640)
        r = hstack(d, top, "Actions", gap=8)
        ibtn(d, r, "Go to files", "folder")
        meta(d, top, ["Links work without signing in", "You can revoke one at any time"])
        return f
    links_toolbar(d, f)
    hud(d, f, "5 links · 2 with a password · opened 27 times in all", 24, 130, color="muted", name="Summary")
    links_table(d, f, 154, hover="Q3 report.docx" if mode == "list" else None)
    d.text(f, "A link opens its item wherever it is now: a moved or renamed file keeps its link. A link whose item "
               "is in the trash or deleted shows the name it was shared with. Revoking a link doesn't touch the "
               "item.", "Small/12", "muted", x=24, y=154 + 42 + 62 * len(LINKS) + 2 + 14, w=W - 48,
           name="Table note")
    if mode == "list":
        toast(d, f, W - 24 - 420, H - 34 - 24 - 70, 420, "good", "Link copied",
              "The link to Q3 report.docx is on your clipboard.")
    if mode == "revoke":
        scrim(d, f, W, H)
        sw, sh = 520, 340
        s, body = sheet(d, f, (W - sw) / 2, area_center_y(H, sh), sw, sh, "Revoke this link?",
                        "Q3 report.docx · made today 14:32")
        item_box(d, body, sw - 48, facts="My files / Documents / Reports · opened 3 times · password on")
        d.text(body, "Anyone who opens it afterwards sees \"This link doesn't work\". The file stays where it is, "
                     "and you can make a new link later; it gets a new address.", "Body/14", "muted", name="Explain",
               w=sw - 48)
        spacer(d, body, v=True)
        sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Cancel"),
                      right=lambda r: ibtn(d, r, "Revoke link", "x", "Danger"))
    return f


def account_menu_frame(d, name, x, y):
    """Where the links page lives: the account menu under the person's name (every web header has it)."""
    f = v2.v2_home(d, name, x, y)
    hid = fix_header(d, f)
    u = d.sub(hid, "User")
    d.set(u, fill="sunken")
    ub = d.R("node.get", {"id": u, "fields": ["absoluteBounds"]})["absoluteBounds"]
    fb = d.R("node.get", {"id": f, "fields": ["absoluteBounds"]})["absoluteBounds"]
    mw = 260
    m = d.frame(f, "Account menu", ub["x"] - fb["x"], 48 + 4, mw, 10, fill="bg", stroke="line", layout="vertical",
                padding=[0, 0, 4, 0], sizingV="hug", clip=False)
    d.effect_style(m, "Shadow/Dialog")
    head = vstack(d, m, "Who", 0, 0, mw, gap=4, pad=[12, 14, 12, 14])
    d.set(head, sizingH="fill")
    side_stroke(d, head, bottom=1)
    d.text(head, OWNER[1], "Body/14 Medium", "ink", name="Name")
    hud(d, head, f"{OWNER[2]} · {OWNER[3]}", color="muted", name="Username")
    new_tag(d, head)
    sp = d.frame(m, "Top pad", 0, 0, mw, 4, fill=None, sizingH="fill")
    del sp
    it = d.inst("Menu item", m, props={"Label": "Your share links", "Icon": False, "Show trail": True, "Trail": "5",
                                       "State": "Hover"}, name="Item Your share links")
    d.set(it, sizingH="fill")
    # SIGN OUT stays its own header cell, so the menu doesn't repeat it.
    return f


# --- 0.1.0: the public share page ------------------------------------------------------------------------

def public_page(d, name, x, y, w=W, h=H):
    f = page(d, name, x, y, w=w, h=h, nav=None, footer_left=f"CraftCloud {VERSION} · {HOST}")
    hid = fix_header(d, f)
    if w != W:
        d.set(hid, width=w)
        ft = d.sub(f, "Footer")
        d.set(ft, width=w)
        d.set(d.sub(ft, "Right"), visible=False)
        d.set(d.sub(hid, "Suite"), visible=False)
        d.set(d.sub(hid, "Wordmark"), visible=False)
    scrim(d, f, w, h)
    return f


def public_sheet(d, f, w, h, sw, sh, title, caption):
    margin = 12 if w < 600 else 32
    sw = min(sw, w - 2 * margin)
    sh = min(sh, h - 48)
    s, body = sheet(d, f, (w - sw) / 2, area_center_y(h, sh), sw, sh, title, caption)
    return s, body, sw


def public_file(d, name, x, y, phone=False):
    w, h = (PHONE_W, PHONE_H) if phone else (W, H)
    f = public_page(d, name, x, y, w, h)
    s, body, sw = public_sheet(d, f, w, h, 560, 330, "Q3 report.docx", "Link works until 2026-11-21")
    p = panel(d, body, "File", sw - 48)
    for k, v in (("Size", "248 KB"), ("Type", KIND["wordcraft"])):
        rr = hstack(d, p, k, gap=0)
        kk = hud(d, rr, k, color="muted", name="k")
        d.set(kk, sizingH="fixed", width=110)
        d.text(rr, v, "Body/14", "ink_strong", name="v")
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: d.text(r, "Shared from CraftCloud", "Small/12.5", "muted", name="From"),
                  right=lambda r: ibtn(d, r, "Download", "download"))
    return f


SHARED_ROOT = [("Photos", None), ("Drafts", None), ("Brief.docx", "248 KB"), ("Moodboard.png", "4.2 MB"),
               ("Schedule.xlsx", "86 KB")]
SHARED_SUB = [("2026", None), ("Shoot-01.jpg", "6.1 MB"), ("Shoot-02.jpg", "5.8 MB"), ("Contact sheet.pdf", "2.4 MB")]


def public_folder(d, name, x, y, sub=False):
    f = public_page(d, name, x, y)
    s, body, sw = public_sheet(d, f, W, H, 560, 520, "Team", "Shared with you")
    d.set(body, gap=0)
    bw = sw - 48
    items = SHARED_SUB if sub else SHARED_ROOT
    if sub:
        cr = hstack(d, body, "Crumbs", gap=6)
        t = d.text(cr, "Team", "Body/13", "accent_ink", name="Crumb Team")
        d.set(t, textDecoration="underline")
        d.text(cr, "/", "Body/13", "muted", name="slash")
        d.text(cr, "Photos", "Body/13", "ink_strong", name="Crumb Photos")
        d.frame(body, "gap8", 0, 0, 10, 8, fill=None)
    hud(d, body, f"{len(items)} items in this folder", color="muted", name="Count")
    d.frame(body, "gap8b", 0, 0, 10, 8, fill=None)
    hover = "Shoot-01.jpg" if sub else "Photos"
    for i, (nm, size) in enumerate(items):
        if i:
            hl = d.rect(body, 0, 0, bw, 1, "line", name="hairline")
            d.set(hl, sizingH="fill")
        r = d.frame(body, f"Row {nm}", 0, 0, bw, 40, fill="sunken" if (nm == hover and size is None) else None,
                    layout="horizontal", gap=8, crossAlign="center", sizingH="fill", sizingV="fixed")
        if size is None:
            t = d.text(r, f"{nm}/", "Body/14", "ink_strong", name="Folder name")
            d.set(t, sizingH="fill")
            o = hud(d, r, "Open", color="accent_ink", name="Open")
            d.set(r, padding=[0, 8, 0, 0])
            del o
        else:
            t = d.text(r, nm, "Body/14", "ink_strong", name="File name")
            d.set(t, sizingH="fill")
            d.text(r, size, "Small/12", "muted", name="Size")
            b = ibtn(d, r, "Download", "download", "Secondary", name="Download")
            if nm == hover:
                d.set(b, fill="invert_bg", strokes=[])
                d.set(d.sub(b, "Label"), fill="invert_fg")
                d.set(d.sub(d.sub(b, "Icon"), "glyph"), stroke="invert_fg")
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: d.text(r, "Shared from CraftCloud", "Small/12.5", "muted", name="From"),
                  right=lambda r: ibtn(d, r, "Download all", "download"))
    return f


def public_password(d, name, x, y, wrong=False):
    f = public_page(d, name, x, y)
    s, body, sw = public_sheet(d, f, W, H, 480, 330, "Enter the password", "Shared link")
    d.set(body, gap=12)
    d.text(body, "The person who shared this link protected it with a password.", "Body/14", "muted",
           name="Explain", w=sw - 48)
    v2.field(d, body, "Password", "•••••", state="Focus", w=sw - 48)
    if wrong:
        d.text(body, "That password isn't right.", "Body/14", "bad", name="Error")
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, right=lambda r: ibtn(d, r, "Open", "arrow-right"))
    return f


def public_dead(d, name, x, y, expired=True):
    f = public_page(d, name, x, y)
    title, text = (("This link has expired", "Ask the person who shared it for a new link.") if expired else
                   ("This link doesn't work", "It was removed, or the address is incomplete. Ask the person who "
                                              "shared it."))
    s, body, sw = public_sheet(d, f, W, H, 480, 260, title, "Shared link")
    d.text(body, text, "Body/14", "muted", name="Explain", w=sw - 48)
    return f


# --- build ------------------------------------------------------------------------------------------------

def build(d):
    """Makes the page "v2 · Sharing" and its frames; returns [(export name, frame id)] like build_v2."""
    d.page(PAGE)
    parts(d)
    frames = []
    col, rowh = W + GAP, H + GAP
    rows = [
        [("v2-web-share-create", lambda n, x, y: create_frame(d, n, x, y)),
         ("v2-web-share-create-light", lambda n, x, y: create_frame(d, n, x, y, light=True)),
         ("v2-web-share-create-date", lambda n, x, y: create_frame(d, n, x, y, state="date")),
         ("v2-web-share-create-error", lambda n, x, y: create_frame(d, n, x, y, state="error"))],
        [("v2-web-share-created", lambda n, x, y: create_frame(d, n, x, y, state="created")),
         ("v2-web-share-created-copied", lambda n, x, y: create_frame(d, n, x, y, state="copied")),
         ("v2-web-share-created-light", lambda n, x, y: create_frame(d, n, x, y, light=True, state="created"))],
        [("v2-web-share-account-menu", lambda n, x, y: account_menu_frame(d, n, x, y)),
         ("v2-web-share-links", lambda n, x, y: links_frame(d, n, x, y)),
         ("v2-web-share-links-revoke", lambda n, x, y: links_frame(d, n, x, y, mode="revoke")),
         ("v2-web-share-links-empty", lambda n, x, y: links_frame(d, n, x, y, mode="empty"))],
        [("v2-web-share-public-file", lambda n, x, y: public_file(d, n, x, y)),
         ("v2-web-share-public-folder", lambda n, x, y: public_folder(d, n, x, y)),
         ("v2-web-share-public-subfolder", lambda n, x, y: public_folder(d, n, x, y, sub=True)),
         ("v2-web-share-public-password", lambda n, x, y: public_password(d, n, x, y))],
        [("v2-web-share-public-password-wrong", lambda n, x, y: public_password(d, n, x, y, wrong=True)),
         ("v2-web-share-public-expired", lambda n, x, y: public_dead(d, n, x, y)),
         ("v2-web-share-public-gone", lambda n, x, y: public_dead(d, n, x, y, expired=False)),
         ("v2-web-share-public-phone", lambda n, x, y: public_file(d, n, x, y, phone=True))],
    ]
    for r, items in enumerate(rows):
        for c, (n, fn) in enumerate(items):
            frames.append((n, fn(n, c * col, r * rowh)))
    return frames
