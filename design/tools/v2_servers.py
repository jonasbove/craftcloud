"""v2 · Servers: the desktop client on a server that isn't CraftCloud's own (docs/WEBDAV-STORAGE.md § 10, § 11).

- The Connect sheet with what the client found at the address, the way on, and "Another kind of server".
- The picker (CraftCloud, Nextcloud, ownCloud, OpenCloud, Other WebDAV server) and the forms it leads to:
  the WebDAV address, user name and password; and the app password (ownCloud) or app token (OpenCloud).
- After the check on connect: the Download-only panel and the Not-possible panel.
- "Download only" beside the state: the header's status cell and the tray's status line.
- Settings › Server, and the "What was checked" sheet its button (and the two panels') opens.

Every frame comes dark and light. The frames are built from v2.py's helpers and components; nothing is
added to the Components page. The check rows are the words of `crates/core/src/storage/probe.rs::checks`.
"""

from screens import hud, vstack, hstack, side_stroke
import v2
from v2 import (DW, DH, HOST, SERVER_URL, page, sheet, sheet_actions, ibtn, btn, spacer, panel, note, scrim,
                steps_cells, field, headline, meta, dot, logo_state, activity_list)

PAGE = "v2 · Servers"
AREA = DH - 48 - 34  # between the header and the footer
SIGNED_IN = f"Signed in as john · {HOST}"

# The picker: the kind and how a person signs in to it (§ 10).
KINDS = [("CraftCloud", "Sign in in the browser"),
         ("Nextcloud", "Sign in in the browser"),
         ("ownCloud", "An app password from Settings › Security"),
         ("OpenCloud", "An app token from Account › App Tokens"),
         ("Other WebDAV server", "The WebDAV address, a user name and a password")]

NOT_ACCOUNT = "not your account password"
PLAIN_HTTP = "A password is never sent over plain http, except to this computer."

DOWNLOAD_ONLY = ("This server overwrote a file it was asked to leave alone, so CraftCloud won't upload to it: files "
                 "come down to this computer, and changes made here stay here.")
NOT_POSSIBLE = ("CraftCloud can't sync with this server safely: it lists files without a version tag, so a change "
                "can't be told from a copy. Ask your server's admin, or use a CraftCloud server.")

# What the check found on a Nextcloud 31 (§ 5), in the words of probe.rs::checks.
CHECKS = [
    ("Create only where nothing is (If-None-Match: *)",
     "honoured: refused (412) when a file was already there, nothing changed"),
    ("Overwrite only the version last seen (If-Match)",
     "honoured: refused (412) when the version tag was stale, nothing changed"),
    ("Delete only the version last seen (If-Match)", "honoured: a stale version tag was refused (412)"),
    ("A move never lands on an existing name (Overwrite: F)", "yes: refused (412), both files unchanged"),
    ("The version tag (ETag) changes on every write", "yes"),
    ("The version tag survives a move", "yes"),
    ("An upload's version tag is the one listings show", "yes"),
    ("A folder's version tag follows its contents", "yes (not relied on: every folder is listed every pass)"),
    ("An upload into a missing folder answers", "404 (the folders are made first)"),
    ("Listing a whole tree at once (Depth: infinity)", "yes"),
    ("Copying on the server (COPY)", "yes"),
    ("Files have ids that survive a move", "yes"),
    ("Checksums in listings", "SHA1"),
    ("Names that differ only in case", "two files"),
    ("Where a delete goes", "the server's own trash"),
    ("Where the bytes an overwrite replaces are kept", "the server's own versions"),
    ("Where the free space comes from", "the server's OCS user endpoint"),
    ("Names the server can't take (left alone by the sync)", "a backslash, a control character, more than 255 bytes"),
]

# Activity under download only (§ 9): what comes down, and what stays here, said once.
ACT = [("14:32", "Down", "Accent", "Documents/Reports/Q3 report.docx", "248 KB"),
       ("14:20", "Down", "Accent", "Design/Poster.psd", "42.1 MB"),
       ("14:05", "Not synced", "Muted", "Notes/Ideas.md",
        "changed on this computer, not uploaded: this server only syncs downwards"),
       ("13:41", "Not synced", "Muted", "Notes/New idea.md", "new on this computer, not uploaded"),
       ("12:05", "Down", "Accent", "Photos/2026/IMG_2041.CR3", "28.6 MB"),
       ("11:47", "Down", "Accent", "Projects/Studio/Floorplan.dwg", "3.4 MB")]


# --- parts ------------------------------------------------------------------------------------------------

def link(d, parent, text, name=None):
    t = d.text(parent, text, "Small/12.5", "accent_ink", name=name or text)
    d.set(t, textDecoration="underline")
    return t


def badge(d, parent, tone, label, name=None):
    return d.inst("Status badge", parent, props={"Tone": tone, "Label": label}, name=name or f"Badge {label}")


def dl_badge(d, parent):
    """The "Download only" badge: the status badge in warn, so it never reads as a two-way sync."""
    return badge(d, parent, "Warn", "Download only", name="Download only badge")


def flow_sheet(d, name, x, y, light, step, labels, title, caption, sh, signed_in=False):
    """A sign-in sheet over the dimmed window, as v2.signin_sheet, in either theme."""
    st, foot = (("accent_ink", "Signed in"), SIGNED_IN) if signed_in else (("faint", "Not connected"), "Not connected")
    f = page(d, name, x, y, w=DW, h=DH, light=light, nav=None, desktop=True, status=st, footer_left=foot)
    scrim(d, f, DW, DH)
    sw = 640
    s, body = sheet(d, f, (DW - sw) / 2, 48 + (AREA - sh) / 2, sw, sh, title, f"Step {step} of 3 · {caption}",
                    right=lambda hd: steps_cells(d, hd, step, labels))
    return f, s, body, sw


def found_row(d, body, tone, label, text):
    r = hstack(d, body, "Found", gap=10)
    badge(d, r, tone, label, name="Found badge")
    d.text(r, text, "Small/12.5", "ink", name="Found text")
    return r


def kind_box(d, body, w, text):
    """The server the form is for, with the way back to the picker."""
    b = d.frame(body, "Kind", 0, 0, w, 10, fill="sunken", stroke="line", layout="horizontal", gap=12,
                padding=[10, 12, 10, 12], crossAlign="center", sizingH="fill", sizingV="hug", clip=False)
    hud(d, b, "Server", color="muted", name="Kind label")
    t = d.text(b, text, "Body/14 Medium", "ink_strong", name="Kind text")
    d.set(t, sizingH="fill")
    link(d, b, "Another kind of server")
    return b


# --- 1. the Connect sheet with the detected server ----------------------------------------------------------

def connect_found(d, name, x, y, light=False):
    f, s, body, sw = flow_sheet(d, name, x, y, light, 1, ("Server", "Browser", "Folders"), "Connect to CraftCloud",
                                "server", 400)
    field(d, body, "Server address", SERVER_URL, state="Focus", w=sw - 48,
          hint="Include http:// or https://. Your admin has it, or the web app's home page.")
    fb = vstack(d, body, "Found block", 0, 0, sw - 48, gap=8)
    d.set(fb, sizingH="fill")
    found_row(d, fb, "Good", "Found", f"Nextcloud 31 at {HOST}")
    note(d, fb, "You sign in on the server's own page in your browser. CraftCloud gets an app password made for "
                "this computer, never your account password.", sw - 48, name="Way on")
    link(d, body, "Another kind of server")
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Quit"),
                  right=lambda r: ibtn(d, r, "Sign in in the browser", "browser"))
    return f


# --- 2. the picker and the forms ------------------------------------------------------------------------------

def connect_kind(d, name, x, y, light=False, found="Nextcloud", sel="Other WebDAV server"):
    f, s, body, sw = flow_sheet(d, name, x, y, light, 1, ("Server", "Sign in", "Folders"), "Connect to CraftCloud",
                                "server", 500)
    bw = sw - 48
    field(d, body, "Server address", SERVER_URL, w=bw)
    blk = vstack(d, body, "Kind block", 0, 0, bw, gap=6)
    d.set(blk, sizingH="fill")
    hud(d, blk, "Kind of server", color="ink", name="Kind of server label")
    t = d.frame(blk, "Kinds", 0, 0, bw, 10, fill="bg", stroke="line", layout="vertical", sizingH="fill",
                sizingV="hug", clip=False)
    for i, (kind, how) in enumerate(KINDS):
        on = kind == sel
        r = d.frame(t, f"Kind {kind}", 0, 0, bw, 40, fill="accent_tint" if on else None, layout="horizontal", gap=10,
                    padding=[0, 12, 0, 12], crossAlign="center", sizingH="fill")
        if i < len(KINDS) - 1:
            side_stroke(d, r, bottom=1)
        slot = d.frame(r, "Mark", 0, 0, 14, 14, fill=None, clip=False)
        if on:
            ck = d.inst("v2/Icon/check", slot, name="Check")
            d.set(d.sub(ck, "glyph"), stroke="accent_ink")
        d.text(r, kind, "Body/14 Medium", "ink_strong" if on else "ink", name="Kind name")
        if kind == found:
            badge(d, r, "Good", "Found", name="Found badge")
        spacer(d, r)
        d.text(r, how, "Small/12", "muted", name="How")
        if on:
            rule = d.rect(r, 0, 0, 2, 40, "accent", name="Chosen rule")
            d.absolute(rule, 0, 0, 2, 40)
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Back"), right=lambda r: ibtn(d, r, "Continue", "arrow-right"))
    return f


def connect_form(d, name, x, y, light=False, kind="webdav"):
    """kind: webdav (address, user name, password) · owncloud (app password) · opencloud (app token)."""
    sh = {"webdav": 520, "owncloud": 444, "opencloud": 444}[kind]
    f, s, body, sw = flow_sheet(d, name, x, y, light, 2, ("Server", "Sign in", "Folders"),
                                {"webdav": "Your WebDAV server", "owncloud": "Sign in to ownCloud",
                                 "opencloud": "Sign in to OpenCloud"}[kind], "sign in", sh)
    bw = sw - 48
    if kind == "webdav":
        kind_box(d, body, bw, "Other WebDAV server")
        field(d, body, "WebDAV address", f"{SERVER_URL}/dav/", w=bw)
        field(d, body, "User name", "john", w=bw)
        fl = field(d, body, "App or WebDAV password", "••••••••••••••••", state="Focus", w=bw,
                   hint=f"An app or WebDAV password, {NOT_ACCOUNT}.")
    elif kind == "owncloud":
        kind_box(d, body, bw, f"ownCloud 10.15 at {HOST}")
        field(d, body, "User name", "john", w=bw)
        fl = field(d, body, "App password", "••••••••••••••••", state="Focus", w=bw,
                   hint=f"Make one in ownCloud under Settings › Security and paste it here: an app password, "
                        f"{NOT_ACCOUNT}.")
    else:
        kind_box(d, body, bw, f"OpenCloud 9.1 at {HOST}")
        field(d, body, "User name", "john", w=bw)
        fl = field(d, body, "App token", "••••••••••••••••", state="Focus", w=bw,
                   hint=f"Make one in OpenCloud under Account › App Tokens and paste it here: an app token, "
                        f"{NOT_ACCOUNT}.")
    note(d, fl, PLAIN_HTTP, bw, name="Plain http note")
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: btn(d, r, "Back"), right=lambda r: ibtn(d, r, "Continue", "arrow-right"))
    return f


# --- 3 and 4. after the check ---------------------------------------------------------------------------------

def check_panel(d, name, x, y, light=False, possible=True):
    title = "Download only" if possible else "Not possible"
    f, s, body, sw = flow_sheet(d, name, x, y, light, 2, ("Server", "Check", "Folders"), title, "the check", 380,
                                signed_in=True)
    bw = sw - 48
    if possible:
        found_row(d, body, "Warn", "Download only", f"OpenCloud 9.1 at {HOST}")
    else:
        found_row(d, body, "Bad", "Not possible", f"WebDAV server at {HOST}")
    d.text(body, DOWNLOAD_ONLY if possible else NOT_POSSIBLE, "Body/14", "ink", name="What it means", w=bw)
    r = hstack(d, body, "Check buttons", gap=8)
    btn(d, r, "What was checked")
    ibtn(d, r, "Check again", "refresh", "Secondary")
    spacer(d, body, v=True)
    if possible:
        sheet_actions(d, s, sw, left=lambda r_: btn(d, r_, "Use another server"),
                      right=lambda r_: ibtn(d, r_, "Continue with download only", "arrow-right"))
    else:
        sheet_actions(d, s, sw, left=lambda r_: btn(d, r_, "Quit"),
                      right=lambda r_: ibtn(d, r_, "Use another server", "arrow-right", "Secondary"))
    return f


# --- 5. "Download only" beside the state ------------------------------------------------------------------------

def status_cell(d, parent, color, text, download_only=True, name="Status"):
    r = hstack(d, parent, name, gap=8)
    dot(d, r, color)
    d.text(r, text, "Body/13", "ink", name="Status text")
    if download_only:
        dl_badge(d, r)
    return r


def header(d, f, color, text):
    """The desktop header (v2.header_comp) drawn out, so the status cell can hold the badge."""
    h = d.frame(f, "Header", 0, 0, DW, 48, fill="bg", layout="horizontal", crossAlign="center", sizingV="fixed")
    side_stroke(d, h, bottom=1)

    def cell(nm, pad=(0, 16, 0, 16), left=False, right=False, gap=8, fixed=None):
        c = d.frame(h, nm, 0, 0, fixed or 10, 48, fill=None, layout="horizontal", padding=list(pad), gap=gap,
                    crossAlign="center", sizingH="fixed" if fixed else "hug", sizingV="fill", clip=False)
        if fixed:
            d.set(c, mainAlign="center")
        if left or right:
            side_stroke(d, c, left=1 if left else 0, right=1 if right else 0)
        return c

    d.inst("Brand/Lockup", cell("Brand", (0, 20, 0, 20), right=True), name="Lockup")
    n = cell("Nav", (0, 12, 0, 12), gap=4, right=True)
    for lab in ("Activity", "Folders", "Settings"):
        d.inst("Nav cell", n, props={"Label": lab, "State": "On" if lab == "Activity" else "Off"}, name=f"Nav {lab}")
    d.frame(h, "Spacer", 0, 0, 10, 48, fill=None, sizingH="fill")
    st = cell("Status", left=True, gap=8)
    dot(d, st, color)
    d.text(st, text, "Body/13", "ink", name="Status text")
    dl_badge(d, st)
    for k, path in (("Minimize", "M0 5 L10 5"), ("Maximize", "M0 0 L10 0 L10 10 L0 10 Z"),
                    ("Close", "M0 0 L10 10 M10 0 L0 10")):
        c = cell(k, (0, 0, 0, 0), left=(k == "Minimize"), fixed=46)
        d.node("vector", c, "glyph", 0, 0, path=path, closed=(k == "Maximize"), fill=None, stroke="muted",
               strokeWeight=1)
    return h


def activity_download_only(d, name, x, y, light=False):
    f = page(d, name, x, y, w=DW, h=DH, light=light, header=False, desktop=True,
             footer_left=f"Download only · checked for changes 2 min ago · {HOST}")
    header(d, f, "good", "Up to date")
    top = vstack(d, f, "Hero", 24, 72, DW - 48, gap=12)
    headline(d, top, "Everything\nis *downloaded*.", style="Display/Hero 40", light=light)
    meta(d, top, ["~/CraftCloud", HOST, "3,412 files", "Open folder"], link="Open folder")
    r = hstack(d, top, "Actions", gap=8)
    ibtn(d, r, "Sync now", "sync")
    ibtn(d, r, "Pause", "pause", "Secondary")
    ibtn(d, r, "Open folder", "folder", "Secondary")
    ty = 72 + d.R("node.get", {"id": top, "fields": ["height"]})["height"] + 20
    hud(d, f, "Activity · today", 24, ty, color="muted", name="List label")
    activity_list(d, f, 24, ty + 22, DW - 48, ACT[:max(3, int((DH - 34 - 16 - ty - 22) // 40))])
    return f


def tray_download_only(d, name, x, y, light=False):
    """The tray menu is drawn by the OS, so there "Download only" is words in the status line; in the
    window's header it is the badge. Below the menu: the header's status cell in each state."""
    w, h, mw = 560, 612, 300
    f = v2.screen(d, name, x, y, w, h, light=light)
    d.set(f, fill="sunken")
    bar = d.frame(f, "System tray", 0, 0, w, 32, fill="bg", layout="horizontal", padding=[0, 12, 0, 12], gap=10,
                  crossAlign="center", mainAlign="max")
    side_stroke(d, bar, bottom=1)
    d.text(bar, "Fri 13 Nov  14:33", "Small/12", "ink", name="Clock")
    sel = d.frame(bar, "Tray icon (open)", 0, 0, 30, 30, fill="line", layout="horizontal", mainAlign="center",
                  crossAlign="center", clip=False)
    logo_state(d, sel, "Synced", 20)
    m = d.frame(f, "Tray menu", w - 12 - mw, 38, mw, 10, fill="bg", stroke="line", layout="vertical",
                padding=[4, 0, 4, 0], sizingV="hug", clip=False)
    d.effect_style(m, "Shadow/Dialog")

    def item(lab, trail=None, disabled=False):
        it = d.inst("Menu item", m, props={"Label": lab, "Icon": False, "Show trail": bool(trail), "State": "Default",
                                           **({"Trail": trail} if trail else {})}, name=f"Item {lab}")
        d.set(it, sizingH="fill")
        if disabled:
            d.set(d.sub(it, "Label"), fill="muted")

    def sep():
        s = d.frame(m, "Separator", 0, 0, mw, 9, fill=None, layout="vertical", padding=[4, 0, 4, 0], sizingH="fill")
        d.set(d.rect(s, 0, 0, mw, 1, "line", name="rule"), sizingH="fill")

    item("CraftCloud · Up to date · Download only", disabled=True)
    sep()
    for lab in ("Open CraftCloud folder", "Open in browser", "Sync now", "Pause syncing"):
        item(lab)
    item("Recent files", "›")
    sep()
    for lab in ("Show CraftCloud", "Settings…", "Quit CraftCloud"):
        item(lab)
    d.text(f, "The OS draws the tray menu, so there it is words: the status line ends with “· Download "
              "only” in every state. The rest of the menu is as on any server.", "Small/12", "muted", x=24, y=52,
           w=w - 12 - mw - 48, name="Tray note")
    y0 = 388
    hud(d, f, "Header status cell · download only", 24, y0, color="muted", name="Cells label")
    box = d.frame(f, "Status cell states", 24, y0 + 22, w - 48, 10, fill="bg", stroke="line", layout="vertical",
                  gap=12, padding=[14, 14, 14, 14], sizingV="hug", clip=False)
    for color, text in (("good", "Up to date"), ("accent_ink", "Syncing 3 of 12"), ("muted", "Paused"),
                        ("bad", "Offline"), ("warn", "1 conflict")):
        status_cell(d, box, color, text, name=f"Cell {text}")
    return f


# --- 6. Settings › Server ---------------------------------------------------------------------------------------

def facts(d, p, rows, kw=100):
    for k, v in rows:
        rr = hstack(d, p, k, gap=0)
        kk = hud(d, rr, k, color="muted", name="k")
        d.set(kk, sizingH="fixed", width=kw)
        d.text(rr, v, "Body/14", "ink", name="v")


def settings_server(d, name, x, y, light=False):
    f = v2.v2_desktop_activity(d, name, x, y, light=light)
    hid = d.sub(f, "Header")
    for lab in ("Activity", "Settings"):
        d.R("instance.setProperties", {"ids": [d.sub(hid, f"Nav {lab}")],
                                       "properties": {"State": "On" if lab == "Settings" else "Off"}})
    scrim(d, f, DW, DH)
    sw, sh = DW - 64, DH - 48
    s, body = sheet(d, f, 32, 24, sw, sh, "Settings", f"CraftCloud {v2.VERSION} · Studio")
    d.set(body, layout="horizontal", gap=24, crossAlign="min")
    cw = (sw - 48 - 24) / 2
    a = vstack(d, body, "Left", 0, 0, cw, gap=14)
    b = vstack(d, body, "Right", 0, 0, cw, gap=14)
    p = panel(d, a, "Account", cw)
    facts(d, p, (("Server", HOST), ("User", "John Doe · john")))
    r, slot = v2.row(d, p, "This device", 200)
    i = d.inst("Input", slot, props={"Value": "Studio"}, name="Device name")
    d.set(i, sizingH="fill")
    ibtn(d, p, "Sign out", "logout", "Secondary")
    p = panel(d, a, "Server", cw)
    facts(d, p, (("Kind", "Nextcloud 31"), ("Address", SERVER_URL), ("Mode", "Two-way"),
                 ("Last check", "18 Nov 2026 at 10:00")))
    r1 = hstack(d, p, "Check buttons", gap=8)
    btn(d, r1, "What was checked")
    ibtn(d, r1, "Check again", "refresh", "Secondary")
    ibtn(d, p, "Open in the browser", "external", "Secondary")
    p = panel(d, b, "On this computer", cw)
    r, slot = v2.row(d, p, "Start at login", 130)
    v2.cells(d, slot, ("Off", "On"), sel="On", name="Login")
    r, slot = v2.row(d, p, "Syncing", 130)
    v2.cells(d, slot, ("Paused", "Active"), sel="Active", name="Syncing")
    r, slot = v2.row(d, p, "Appearance", 200)
    v2.cells(d, slot, ("System", "Dark", "Light"), sel="System", name="Theme")
    p = panel(d, b, "About", cw)
    meta(d, p, [f"CraftCloud {v2.VERSION}", "Linux x86_64"])
    meta(d, p, ["MIT OR Apache-2.0"])
    meta(d, p, ["Source", "Release notes", "Report a problem"], link="Source")
    note(d, p, "Look based on ArtCraft Launcher by the ArtCraft team.", cw - 28)
    return f


def checked_sheet(d, name, x, y, light=False):
    """"What was checked": the probe's table in plain words, one row per check, in a list that scrolls."""
    f = v2.v2_desktop_activity(d, name, x, y, light=light)
    hid = d.sub(f, "Header")
    for lab in ("Activity", "Settings"):
        d.R("instance.setProperties", {"ids": [d.sub(hid, f"Nav {lab}")],
                                       "properties": {"State": "On" if lab == "Settings" else "Off"}})
    scrim(d, f, DW, DH)
    sw, sh = DW - 64, DH - 48
    s, body = sheet(d, f, 32, 24, sw, sh, "What was checked", f"Nextcloud 31 · {HOST}",
                    right=lambda hd: badge(d, hd, "Good", "Two-way", name="Mode badge"))
    d.set(body, gap=12, padding=[16, 24, 16, 24])
    bw = sw - 48
    meta(d, body, ["Checked 18 Nov 2026 at 10:00", f"{len(CHECKS)} checks", "Mode: two-way"])
    note(d, body, "The check writes only in a folder of its own on the server (.craftcloud/probe/) and removes "
                  "it afterwards. It never touches your files.", bw, name="Check note")
    row_h, name_w = 32, 352
    th = sh - 84 - 62 - 32 - 14 - 18 - 24 - 2  # what is left for the list
    t = d.frame(body, "Checks", 0, 0, bw, th, fill="bg", stroke="line", layout="vertical", padding=[0, 16, 0, 16],
                sizingH="fill", sizingV="fixed", clip=True)
    hd = d.frame(t, "Header row", 0, 0, bw - 32, row_h, fill=None, layout="horizontal", crossAlign="center",
                 sizingH="fill")
    side_stroke(d, hd, bottom=1)
    c = hud(d, hd, "Check", color="muted", name="H check")
    d.set(c, sizingH="fixed", width=name_w)
    hud(d, hd, "What the server did", color="muted", name="H result")
    for i, (check, result) in enumerate(CHECKS):
        r = d.frame(t, f"Check {i + 1}", 0, 0, bw - 32, row_h, fill=None, layout="horizontal", crossAlign="center",
                    sizingH="fill")
        side_stroke(d, r, bottom=1)
        n = d.text(r, check, "Body/13", "ink", name="Check")
        d.set(n, sizingH="fixed", width=name_w)
        d.text(r, result, "Small/12", "muted", name="Result")
    track = d.rect(t, 0, 0, 10, th - 2, "accent_tint", name="Scroll track")
    d.absolute(track, bw - 11, 1, 10, th - 2)
    bar = d.rect(t, 0, 0, 10, 190, "accent", name="Scroll bar")
    d.absolute(bar, bw - 11, 1, 10, 190)
    sheet_actions(d, s, sw, left=lambda r_: ibtn(d, r_, "Check again", "refresh", "Secondary"),
                  right=lambda r_: ibtn(d, r_, "Done", "check"))
    return f


# --- build ------------------------------------------------------------------------------------------------

def build(d):
    """Makes the page "v2 · Servers" and its frames; returns [(export name, frame id)] like build_v2."""
    d.page(PAGE)
    frames = []
    col, rowh = DW + 120, DH + 120
    rows = [
        [("v2-desktop-connect-found", connect_found),
         ("v2-desktop-connect-kind", connect_kind),
         ("v2-desktop-connect-webdav", connect_form),
         ("v2-desktop-connect-app-password", lambda *a, **k: connect_form(*a, kind="owncloud", **k)),
         ("v2-desktop-connect-app-token", lambda *a, **k: connect_form(*a, kind="opencloud", **k))],
        [("v2-desktop-check-download-only", check_panel),
         ("v2-desktop-check-not-possible", lambda *a, **k: check_panel(*a, possible=False, **k)),
         ("v2-desktop-activity-download-only", activity_download_only),
         ("v2-desktop-tray-download-only", tray_download_only)],
        [("v2-desktop-settings-server", settings_server),
         ("v2-desktop-settings-checked", checked_sheet)],
    ]
    # Dark rows, then the same rows in light under them.
    for light in (False, True):
        for r, items in enumerate(rows):
            for c, (n, fn) in enumerate(items):
                nm = f"{n}-light" if light else n
                frames.append((nm, fn(d, nm, c * col, (r + (len(rows) if light else 0)) * rowh, light=light)))
    return frames
