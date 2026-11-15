"""v2 · Single sign-on: Admin › Server › Single sign-on and the sign-in sheet with the provider's button.

What 0.1.0 does is the truth (crates/ui-egui/src/web/admin.rs `sso_ui`, web/oidc.rs `sso_sign_in_ui`,
web.rs `login_ui`, crates/core/src/api/oidc.rs `login_error_text`, docker/Caddyfile's `login_error` keys);
anything that isn't there yet carries the "New in 0.2.0" tag.
"""

from screens import hud, vstack, hstack, screen
from v2 import (W, H, GAP, HOST, SERVER_URL, page, scrim, sheet, sheet_actions, field, btn, ibtn, cells, panel,
                row, spacer, square, toolbar)

PAGE = "v2 · Single sign-on"
PROVIDER = "Authentik"
ISSUER = "https://id.example.org/application/o/craftcloud/"
CALLBACK = f"{SERVER_URL}/api/auth/oidc/callback"
SECRET = "•" * 24


def new_tag(d, parent, name="New in 0.2.0"):
    """The "New in 0.2.0" HUD tag: something this frame shows that 0.1.0 doesn't do yet."""
    return d.inst("Status badge", parent, props={"Tone": "Accent", "Label": "New in 0.2.0"}, name=name)


# --- Admin › Server › Single sign-on -------------------------------------------------------------------------

# Every frame's form: (enabled, auto_provision, password_login), saved or typed, and what the bottom row says.
STATES = {
    # The saved, working setup.
    "saved": dict(badge=("Good", f"On · {PROVIDER}"), toggles=(True, True, True), secret=None, dirty=False,
                  status=("good", "Saved. Single sign-on uses the new settings now.")),
    # First setup, typed but not saved; Test connection asked.
    "testing": dict(badge=("Muted", "Off"), toggles=(True, True, True), secret=SECRET, dirty=True, test="testing",
                    status=("warn", "Not saved yet.")),
    "works": dict(badge=("Muted", "Off"), toggles=(True, True, True), secret=SECRET, dirty=True, test="works",
                  status=("warn", "Not saved yet.")),
    "fails": dict(badge=("Muted", "Off"), toggles=(True, True, True), secret=SECRET, dirty=True, test="fails",
                  status=("warn", "Not saved yet."), issuer="https://id.example.org/application/o/craftcloud"),
    # Save with password sign-in off while the provider is off: refused (admin.rs, 0.1.0).
    "guard": dict(badge=("Muted", "Off"), toggles=(False, True, False), secret=None, dirty=True,
                  status=("bad", "Password sign-in can only be switched off while sign-in with the provider is on.")),
    # Save with password sign-in off while the provider doesn't answer: refused (new).
    "guard-untested": dict(badge=("Good", f"On · {PROVIDER}"), toggles=(True, True, False), secret=None, dirty=True,
                           test="fails", new_guard=True,
                           status=("bad", "Password sign-in stays on until Test connection works: with the provider "
                                          "out of reach, nobody could sign in.")),
    # Settings from OXICLOUD_OIDC_* in .env: shown, locked.
    "env": dict(badge=("Good", f"On · {PROVIDER}"), toggles=(True, False, True), secret=None, dirty=False,
                env={"Issuer URL": "ISSUER_URL", "Client ID": "CLIENT_ID", "Client secret": "CLIENT_SECRET",
                     "Create accounts at first sign-in": "AUTO_PROVISION"}, status=None),
}


def sso_field(d, parent, label, value, hint, w, env=None):
    fl = field(d, parent, label, value, w=w, hint=f"Set by OXICLOUD_OIDC_{env}." if env else hint)
    if env:
        d.set(d.sub(fl, f"{label} row"), opacity=0.5)
    return fl


def v2_admin_sso(d, name, x, y, state="saved", light=False):
    st = STATES[state]
    env = st.get("env", {})
    f = page(d, name, x, y, light=light, nav="Admin")
    tb = toolbar(d, f, 24, 72, W - 48)
    square(d, tb, "refresh", "Refresh")
    cells(d, tb, ("Users", "Server"), sel="Server", name="Sub-nav")
    spacer(d, tb)
    hud(d, tb, "Server · below the web apps", color="muted", name="Where")

    sec = vstack(d, f, "Single sign-on", 24, 126, W - 48, gap=8)
    hr = hstack(d, sec, "Section head", gap=8)
    hud(d, hr, "Single sign-on", color="ink", name="Section title")
    d.inst("Status badge", hr, props={"Tone": st["badge"][0], "Label": st["badge"][1]}, name="State badge")
    d.text(sec, "Let people sign in with an OpenID Connect identity provider. docs/OIDC.md shows how to register "
                "CraftCloud there.", "Body/13", "muted", name="Intro")

    box = vstack(d, sec, "Form", 0, 0, W - 48, gap=10, pad=[16, 16, 16, 16])
    d.set(box, sizingH="fill", strokes=[{"type": "solid", "color": "#fff"}], strokeWeight=1, strokeAlign="inside")
    d.R("variable.bind", {"ids": [box], "field": "strokes/0/color", "variable": "line"})
    d.set(sec, gap=8)
    cw = (W - 48 - 32 - 24) / 2
    two = hstack(d, box, "Columns", gap=24)
    d.set(two, sizingH="fill", crossAlign="min")
    left = vstack(d, two, "Left", 0, 0, cw, gap=10)
    right = vstack(d, two, "Right", 0, 0, cw, gap=10)
    d.set([left, right], sizingH="fill")
    secret_hint = "Not set yet." if st["secret"] else "Stored. Type a new one to replace it."
    issuer = st.get("issuer", ISSUER)
    for label, value, hint in (("Button name", PROVIDER, "Shown as “Sign in with …”."),
                               ("Issuer URL", issuer, "For example https://id.example.org"),
                               ("Client ID", "craftcloud", None),
                               ("Client secret", st["secret"] or " ", secret_hint)):
        sso_field(d, left, label, value, hint, cw, env=env.get(label))
    for label, value, hint in (("Scopes", "openid profile email groups",
                                "Usually openid profile email (add groups for admin groups)."),
                               ("Admin groups", "craftcloud-admins",
                                "Members of these groups (comma-separated) become admins.")):
        sso_field(d, right, label, value, hint, cw, env=env.get(label))
    for label, on in zip(("Sign-in with the provider", "Create accounts at first sign-in", "Password sign-in"),
                         st["toggles"]):
        r, slot = row(d, right, label, 120)
        c = cells(d, slot, ("Off", "On"), sel="On" if on else "Off", name="Switch", disabled=label in env)
        del c
        if label in env:
            d.text(right, f"Set by OXICLOUD_OIDC_{env[label]}.", "Small/12", "muted", name="Locked")

    reg = panel(d, box, "Register with your provider", W - 48 - 32)
    for k, v in (("Redirect URI", CALLBACK), ("Sign-out", "not used")):
        rr = hstack(d, reg, k, gap=0)
        kk = hud(d, rr, k, name="k")
        d.set(kk, sizingH="fixed", width=110)
        d.text(rr, v, "Body/14", "ink_strong", name="v")
    d.text(box, "The redirect URI follows CRAFTCLOUD_PUBLIC_URL. Settings from OXICLOUD_OIDC_* in .env win over "
                "these and can't be changed here.", "Small/12", "muted", name="Env note")

    # The test answer, on a line of its own.
    test = st.get("test")
    if test == "works":
        d.text(box, f"The server reached the provider ({ISSUER}).", "Body/13", "good", name="Test answer")
    elif test == "fails":
        tr = hstack(d, box, "Test answer", gap=10)
        d.text(tr, "The server can't reach id.example.org: the name doesn't resolve from inside the CraftCloud "
                   "container. Check the issuer URL, or give the provider a name the server can look up.",
               "Body/13", "bad", name="Reason")
        new_tag(d, tr)
        d.text(box, "Server said: error sending request for url (https://id.example.org/application/o/craftcloud/"
                    ".well-known/openid-configuration): dns error", "Mono/12", "muted", name="Server words")
    elif test == "testing":
        tr = hstack(d, box, "Test answer", gap=10)
        hud(d, tr, "Asking id.example.org for its OpenID configuration…", color="accent_ink",
            name="Testing")
        new_tag(d, tr)

    bot = hstack(d, box, "Actions", gap=8)
    d.set(bot, sizingH="fill")
    if st["status"]:
        tone, text = st["status"]
        d.text(bot, text, "Body/13", tone, name="Status")
        if st.get("new_guard"):
            new_tag(d, bot, name="New guard")
    spacer(d, bot)
    busy = test == "testing"
    if st["dirty"]:
        btn(d, bot, "Undo", state="Disabled" if busy else "Default")
    btn(d, bot, "Testing…" if busy else "Test connection", state="Disabled" if busy else "Default")
    ibtn(d, bot, "Save single sign-on", "check", state="Disabled" if (busy or not st["dirty"]) else None)
    return f


# --- the sign-in sheet --------------------------------------------------------------------------------------

ERRORS = [
    ("callback_denied · no account here, or a wrong secret",
     "The identity provider's answer was refused. Try again; if it keeps happening, you may not have an account "
     "here yet, or an admin needs to check the single sign-on settings."),
    ("provider_denied · not allowed, or cancelled",
     "The identity provider didn't sign you in (cancelled, or this account isn't allowed to use CraftCloud)."),
    ("provider_unreachable",
     "This server can't reach its identity provider right now. Try again later, or ask an admin to check the "
     "issuer URL."),
    ("callback_failed",
     "Single sign-on didn't complete. Try again, or ask an admin to check the server log."),
    ("The one-time code expired (exchange 401/403)",
     "That single sign-on link expired (they last a minute). Try again."),
    ("email_not_verified_at_idp",
     "Your identity provider hasn't confirmed your email address. Verify it there, then try again."),
]


def sign_in_sheet(d, parent, x, y, password=True, error=None, sw=480):
    """web.rs login_ui: 370 tall, +84 with the provider and the password form, −60 provider only, +30 for an
    error (the sheet here grows with a long error, so nothing clips)."""
    sh = 370 + (84 if password else -60) + (54 if error else 0)
    s, body = sheet(d, parent, x, y, sw, sh, "Sign in", HOST)
    d.set(body, gap=0)
    if error:
        d.text(body, error, "Body/14", "bad", name="Error", w=sw - 48)
        d.frame(body, "Gap", 0, 0, 10, 10, fill=None)
    b = ibtn(d, body, f"Sign in with {PROVIDER}", "arrow-right", "Secondary" if password else "Primary",
             name="Provider button")
    d.set(b, sizingH="fill")
    if password:
        d.frame(body, "Gap", 0, 0, 10, 14, fill=None)
        dv = hstack(d, body, "Divider", gap=12)
        d.set(dv, sizingH="fill")
        for i in (0, 1):
            ln = d.rect(dv, 0, 0, 10, 1, "line", name="rule")
            d.set(ln, sizingH="fill")
            if i == 0:
                hud(d, dv, "or", color="muted", name="Or")
        d.frame(body, "Gap", 0, 0, 10, 10, fill=None)
        field(d, body, "Username", "john", w=sw - 48)
        d.frame(body, "Gap", 0, 0, 10, 14, fill=None)
        field(d, body, "Password", " ", w=sw - 48)
    else:
        d.frame(body, "Gap", 0, 0, 10, 10, fill=None)
        d.text(body, "This server signs people in through its identity provider only.", "Small/12.5", "muted",
               name="Only", w=sw - 48)
    spacer(d, body, v=True)
    sheet_actions(d, s, sw, left=lambda r: d.text(r, "Forgot it? Ask an admin to reset it.", "Small/12", "muted",
                                                  name="Forgot"),
                  right=(lambda r: ibtn(d, r, "Sign in", "arrow-right", state="Disabled")) if password else None)
    return s, sh


def v2_login_sso(d, name, x, y, password=True, light=False, error=None):
    f = page(d, name, x, y, nav=None, light=light)
    scrim(d, f, W, H)
    sh = 370 + (84 if password else -60) + (54 if error else 0)
    sign_in_sheet(d, f, (W - 480) / 2, (H - sh) / 2 - 10, password=password, error=error)
    return f


def v2_login_sso_busy(d, name, x, y):
    """oidc.rs sso_busy_ui: on the way to the provider (and, with "Checking the answer…", on the way back)."""
    f = page(d, name, x, y, nav=None)
    scrim(d, f, W, H)
    sw, sh = 480, 260
    s, body = sheet(d, f, (W - sw) / 2, (H - sh) / 2 - 10, sw, sh, "Signing you in…", f"To {PROVIDER}")
    d.text(body, f"Opening {PROVIDER}.", "Body/14", "muted", name="Body")
    return f


def v2_login_sso_errors(d, name, x, y):
    """Every plain-language error the sign-in sheet shows, each under its login_error key."""
    sw, gap, pad = 480, 32, 32
    sh = 370 + 84 + 54
    fw = pad * 2 + 3 * sw + 2 * gap
    fh = pad + 2 * (28 + sh) + gap + pad
    f = screen(d, name, x, y, fw, fh)
    d.set(f, fill="sunken")
    for i, (key, text) in enumerate(ERRORS):
        r, c = divmod(i, 3)
        xx, yy = pad + c * (sw + gap), pad + r * (28 + sh + gap)
        hud(d, f, key, xx, yy, color="muted", name=f"Key {i}")
        sign_in_sheet(d, f, xx, yy + 28, error=text)
    return f


# --- build --------------------------------------------------------------------------------------------------

def build(d, x0=0, y0=0):
    """The page "v2 · Single sign-on"; returns [(export name, frame id)] like v2.build_v2."""
    d.page(PAGE)
    col, rowh = W + GAP, H + GAP
    items = [
        ("v2-web-admin-sso", lambda n, x, y: v2_admin_sso(d, n, x, y)),
        ("v2-web-admin-sso-light", lambda n, x, y: v2_admin_sso(d, n, x, y, light=True)),
        ("v2-web-admin-sso-testing", lambda n, x, y: v2_admin_sso(d, n, x, y, "testing")),
        ("v2-web-admin-sso-works", lambda n, x, y: v2_admin_sso(d, n, x, y, "works")),
        ("v2-web-admin-sso-fails", lambda n, x, y: v2_admin_sso(d, n, x, y, "fails")),
        ("v2-web-admin-sso-fails-light", lambda n, x, y: v2_admin_sso(d, n, x, y, "fails", light=True)),
        ("v2-web-admin-sso-guard", lambda n, x, y: v2_admin_sso(d, n, x, y, "guard")),
        ("v2-web-admin-sso-guard-untested", lambda n, x, y: v2_admin_sso(d, n, x, y, "guard-untested")),
        ("v2-web-admin-sso-env", lambda n, x, y: v2_admin_sso(d, n, x, y, "env")),
        ("v2-web-login-sso", lambda n, x, y: v2_login_sso(d, n, x, y)),
        ("v2-web-login-sso-light", lambda n, x, y: v2_login_sso(d, n, x, y, light=True)),
        ("v2-web-login-sso-only", lambda n, x, y: v2_login_sso(d, n, x, y, password=False)),
        ("v2-web-login-sso-error", lambda n, x, y: v2_login_sso(d, n, x, y, error=ERRORS[2][1])),
        ("v2-web-login-sso-busy", lambda n, x, y: v2_login_sso_busy(d, n, x, y)),
        ("v2-web-login-sso-errors", lambda n, x, y: v2_login_sso_errors(d, n, x, y)),
    ]
    frames = []
    for i, (n, fn) in enumerate(items):
        r, c = divmod(i, 4)
        frames.append((n, fn(n, x0 + c * col, y0 + r * rowh)))
    return frames
