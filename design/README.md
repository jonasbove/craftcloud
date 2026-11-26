# CraftCloud UI design (v2)

`craftcloud.figcraft` is a real FigCraft document, built with FigCraft 0.18.1 (storytold/figcraft). Every layer,
component, style and variable in it was made through FigCraft's own engine commands; nothing writes the `.figcraft`
JSON directly. The PNGs in `exports/` were exported by FigCraft at 1x, with 2x copies in `exports/2x/`.

**v2 (2026-11-13)** replaces every earlier screen.

- **Source of truth:** these frames are what the egui code is rebuilt from.
- **Layout language:** the ArtCraft Launcher's (storytold/craft-launcher, `crates/ui-egui`). Where it and the website
  differ, the launcher wins.
- **Details from getartcraft.com:** the two-line headline with one accent word, the mono meta lines with middots,
  and leading icons in buttons.
- **Brand:** the mackerel logo and a sage accent.
- **Archive:** all earlier frames are on the page **Archive (pre-v2)**, with 1x exports in `exports/archive/`. Note that the
  archive is regenerated from the same scripts, so it now shows the green accent and John Doe too.

## Pages and frames

| Page | Frames |
|---|---|
| Tokens & type | colour tokens (dark, light), type specimens; `exports/_tokens.png` |
| Components | the library (launcher components plus the v2 ones below); `exports/_components.png` (no background: view on dark) |
| **v2 · Web** (1440×900) | `v2-web-setup-1` … `v2-web-setup-5`, `v2-web-login`, `v2-web-login-device`, `v2-web-device-code`, `v2-web-device` (approve), `v2-web-device-connected`, `v2-web-device-denied`, `v2-web-home`, `v2-web-home-light`, `v2-web-admin-users`, `v2-web-admin-users-new`, `v2-web-admin-users-reset`, `v2-web-admin-users-reset-done`, `v2-web-admin-user-details`, `v2-web-admin-server`, `v2-web-admin-server-slim` |
| **v2 · Desktop** (960×640 window) | `v2-desktop-connect`, `v2-desktop-browser`, `v2-desktop-choose`, `v2-desktop-activity`, `v2-desktop-activity-light`, `v2-desktop-activity-syncing`, `v2-desktop-conflict`, `v2-desktop-folders`, `v2-desktop-settings`, `v2-desktop-states`, `v2-desktop-tray` (560×1000: tray strip, menu with Recent files open, the menu paused and with the folder missing, icon states, window icon) |
| **v2 · Files** (1440×900, phone 390×844) | `v2-web-files`, `v2-web-files-light`, `v2-web-files-empty`, `v2-web-files-loading`, `v2-web-files-details` (new in 0.2.0), `v2-web-files-menu`, `v2-web-files-selected`, `v2-web-files-drop`, `v2-web-files-upload`, `v2-web-files-upload-taken`, `v2-web-files-new-folder`, `v2-web-files-rename`, `v2-web-files-move`, `v2-web-files-versions` (new in 0.2.0), `v2-web-files-trash`, `v2-web-files-trash-delete`, `v2-web-files-trash-taken`, `v2-web-files-recent`, `v2-web-files-starred`, `v2-web-files-search`, `v2-web-files-phone`, `v2-web-files-phone-light` |
| **v2 · Sharing** (1440×900; phone 390×844) | **New in 0.2.0:** `v2-web-share-create`, `v2-web-share-create-light`, `v2-web-share-create-date`, `v2-web-share-create-error`, `v2-web-share-created`, `v2-web-share-created-copied`, `v2-web-share-created-light`, `v2-web-share-account-menu`, `v2-web-share-links`, `v2-web-share-links-revoke`, `v2-web-share-links-empty`. **0.1.0:** `v2-web-share-public-file`, `v2-web-share-public-folder`, `v2-web-share-public-subfolder`, `v2-web-share-public-password`, `v2-web-share-public-password-wrong`, `v2-web-share-public-expired`, `v2-web-share-public-gone`, `v2-web-share-public-phone` (built by `tools/v2_sharing.py`) |
| **v2 · Single sign-on** (1440×900) | `v2-web-admin-sso`, `v2-web-admin-sso-light`, `v2-web-admin-sso-testing`, `v2-web-admin-sso-works`, `v2-web-admin-sso-fails`, `v2-web-admin-sso-fails-light`, `v2-web-admin-sso-guard`, `v2-web-admin-sso-guard-untested`, `v2-web-admin-sso-env`, `v2-web-login-sso`, `v2-web-login-sso-light`, `v2-web-login-sso-only`, `v2-web-login-sso-error`, `v2-web-login-sso-busy`, `v2-web-login-sso-errors` (1568×1168: every sign-in error) |
| **v2 · Apps** (1440×900; phone 390×844) | **New in 0.3.0:** `v2-web-apps-menu`, `v2-web-apps-menu-light`, `v2-web-apps-menu-phone` (built by `tools/v2_apps.py`) |
| Archive (pre-v2) | the 41 earlier frames (launcher-era web frontend, the 2026-11-13 scope, the answers round) |

What each screen shows:

- **Setup:** a launcher sheet over the dimmed page, with the step cells `01 … 05` in the sheet hero.
  - Step 4 is the app grid in a near-full sheet (window − 64 × window − 48, sheet.rs:15): click a card to switch it,
    with Off/On, ALL/NONE, an accent scroll bar, and the `-apps` size as a meta line.
- **Login:** a sheet. The device variant adds a "Studio wants to connect" notice.
- **Device approval:** sheets for the code entry, the approval (device facts plus the code in mono 28), Connected and
  Denied.
- **Home:**
  - The hero headline "Your **cloud**, ready.", a body paragraph and a meta line.
  - Three launcher panels: storage with a 4 px meter; devices Laptop and Studio with a dot and when each was last seen;
    and the desktop client for Linux, macOS and Windows, each with a one-line note on how to open the unsigned build.
- **Admin users:**
  - The toolbar row and the launcher table, with the "…" menu open on Jane.
  - New user and reset password are sheets. The reset-done frame shows the success toast.
  - The user details card follows details.rs.
- **Admin server:** a facts strip (CraftCloud, OxiCloud 0.10.0, public URL, versions kept 5), a toolbar with the web
  apps Off/On, All/None and SAVE, the settings.env note, and the app grid.
  - The slim variant has everything disabled, with a "Needs the -apps image" badge and all cards off.
- **Sharing** (`tools/v2_sharing.py`). Frames marked with the "New in 0.2.0" tag (1 px accent border, Badge 10 in
  accent_ink) are what 0.2.0 is built to (issues #18 and #19); the rest is the public page as 0.1.0 has it.
  - **Share link sheet** (new): 560 wide, opened from a file's or folder's context menu ("Share link…") or its details
    panel ("Share"), over Files. Hero "Share link", caption "<name> · anyone with the link can open it", the tag in the
    hero's right slot. Body, top to bottom, 16 apart: the item box (sunken, hairline, padding 10 12: 20 px file icon in
    the app colour, name Body 14 Medium, then "<format> · <size> · in <folder path>" Small 12 muted); LINK EXPIRES
    (HUD ink) over the cells Never / 1 day / 7 days / 30 days / On a date… (7 days chosen by default) and a hint
    with the end date ("The link stops working on 21 Nov 2026 at 14:32."; Never: "The link works until you revoke
    it."); PASSWORD + OPTIONAL (muted) over an input and a Secondary SHOW / HIDE icon button (eye), then the hint "The
    person opening the link will need it. Send it to them separately, not in the same message as the link." Actions:
    CANCEL (Secondary) left, CREATE LINK (Primary, link icon) right.
  - **On a date…** shows a 200 px date input (focus) and a 34 px calendar square; the picker drops 4 px under the
    input: month row (28 px prev/next squares, "November 2026" Body 14 Medium), Monday-first weekday row (Badge 10
    muted), 34 × 30 day cells (past days muted and not pickable, today boxed in line_strong, the pick inverted), and
    "Past days can't be picked." The hint then reads "The link stops working at the end of 28 Nov 2026."
  - **Error:** the form keeps what was typed (shown here with the password after SHOW); above the actions a sunken
    box with a NOT MADE badge (bad) and one plain sentence in bad ("The server didn't make the link: you don't have
    permission to share this file. Nothing was shared. Try again, or ask your admin."). The primary becomes TRY AGAIN.
  - **Link ready:** LINK over a read-only input with the URL in Mono 12 (truncated with an ellipsis) and a Primary
    COPY (copy icon); after a click it reads COPIED with a check, and the "Link copied" toast (good rule) shows bottom
    right. Then the panel THIS LINK (Shares, Expires, Password "On · it isn't shown again", a lock and "Send the
    password separately."). Actions: "See all your share links" (accent_ink, underlined) left, DONE (Secondary) right.
  - **Your share links** (new) is opened from the **account menu**: clicking the person's name in the header opens a
    260 px menu (name, username · email, then "Your share links" with the count as trail). SIGN OUT stays its own
    cell. The page has no nav cell lit and the name cell sunken. Toolbar: refresh, the filter cells All / Active /
    Expired / No longer there with counts, the tag, search "Search links", SORT Newest. Summary meta line, then the
    table (header 42, rows 62): Shared item (20 px icon, name Body 16 Medium, where it is now in Small 12 muted; a link
    whose item is gone shows the name it was shared with in muted, a NO LONGER THERE badge (warn) and "Shared as … ·
    moved to the trash or deleted"), Expires 170 (date plus "in 7 days", "Never" muted, or an EXPIRED badge over the
    date), Password 110 (lock + Yes, or No muted), Opened 100 (Mono 12), Made 120 (sorted), actions 230: COPY
    (Secondary, copy icon; disabled when expired or gone) and REVOKE (Secondary). Copy shows the "Link copied" toast.
  - **Revoke** is a 520 sheet: "Revoke this link?", the item box, what happens in plain words, CANCEL and REVOKE LINK
    (Danger, x icon). **Empty:** kicker, the headline "No share *links* yet.", how to make one, GO TO FILES.
  - **Public page** (`/s/<token>`, share.rs, no sign-in; header without nav or user): sheets over the scrim, centred
    between header and footer. File 560 × 330 (title the name, caption "Link works until <date>" or "Shared with
    you", panel FILE with Size and Type, "Shared from CraftCloud" and DOWNLOAD). Folder 560 × 520 (count HUD, rows 40:
    folders "Name/" with OPEN in accent_ink, files with size and a Secondary DOWNLOAD; breadcrumbs only below the
    shared folder; DOWNLOAD ALL). Password 480 × 330 ("That password isn't right." in bad). Expired and removed links
    480 × 260. Phone (390 wide): the sheet is the window less 12 px a side, the header keeps the logo and the gear.
    The Type fact names the format ("DOCX document"); 0.1.0 still says "DOCX file".
- **Apps menu** (`tools/v2_apps.py`, new in 0.3.0; `crates/ui-egui/src/web/apps_menu.rs`): when the server hosts
  the Craft web apps (`/craftcloud/config.json`: `web_apps` on and `enabled_apps` not empty), a signed-in header
  gets **APPS** between FILES and ADMIN: the app-grid glyph (nine 2.5 px squares in 14 px), a 6 px gap and the
  HUD label, padding 0 8; on a phone (under 520 px) the glyph alone in a 24 px square. It is a button, not a page:
  never inverted, sunken with ink_strong while hovered or open. With the apps off, or none switched on, the header
  is as before.
  - A click drops the menu 4 px under the header, its left edge on the APPS cell (kept 8 px inside the window):
    bg, 1 px line_strong, the dialog shadow, 400 wide (the window less 16 on a phone). Head 40 (padding 0 14, a
    hairline under it): "CRAFT APPS IN THE BROWSER" HUD ink, the count HUD muted at the right. Grid padding 8,
    four tiles to a row, 96 × 88 (the width shared on a phone): the 40 px app icon 14 from the top, the name Small
    12 ink 8 under it, centred, one line. Hover and focus: sunken, a 2 px rule along the top in the app's colour,
    the name ink_strong; focus adds a 2 px accent_ink ring inside. Foot 34 (hairline on top): "Each app opens in a
    new tab." Small 12 muted and the external-link glyph at the right.
  - A tile opens `/apps/<id>/` in a new tab and closes the menu. Esc, a click outside or the focus moving out of
    it closes it; the arrow keys, Home and End move between tiles.
- **Desktop:**
  - The sign-in screens are sheets over the dimmed window, with the step cells.
  - Activity is the hero (40 px headline, meta line, SYNC NOW / PAUSE / OPEN FOLDER) above the launcher's list frame
    with 20 px status badges UP / DOWN / CONFLICT / DELETED.
    - The syncing variant adds the launcher progress bar: caption, then a 4 px track with an accent fill.
  - Conflict is a sheet over Activity. It shows both versions as panels, the conflict-copy name, and KEEP CLOUD /
    KEEP MINE / KEEP BOTH (primary).
  - Folders is the toolbar row plus a folder table with Off/On cells, followed by the note about unticking.
  - Settings is a sheet over Activity, opened by the SETTINGS header cell, with the panels Account, On this computer
    and About, using the settings rows.
  - The states frame has 5 compact panels (paused, offline, signed out, mass delete, folder missing) plus every state
    of the header status cell.
  - The tray follows `apps/craftcloud/src/tray.rs`: a disabled status line ("CraftCloud · " and the header's status
    text), Open CraftCloud folder, Open in browser, Sync now, Pause / Resume syncing, a **Recent files** submenu (the
    last 5 file names, newest first), then Show CraftCloud, Settings… and Quit CraftCloud, with no shortcuts. With
    the folder missing, Open CraftCloud folder and Sync now are off; "Resume syncing" shows only while paused. The
    icon has four looks: up to date, syncing, paused (also signed out and mass delete, whose menu still says Pause
    syncing) and error (also offline and folder missing).
- **Single sign-on** (page v2 · Single sign-on):
  - **Admin › Server › Single sign-on** (below the web apps, `admin.rs` `sso_ui`): the state badge, two columns
    (button name, issuer URL, client ID, client secret masked / scopes, admin groups and the Off/On rows for
    sign-in with the provider, account creation at first sign-in and password sign-in), the "Register with your
    provider" facts, then Undo / Test connection / Save single sign-on. Frames: saved (dark and light), testing,
    works, fails (dark and light), the 0.1.0 guard ("Password sign-in can only be switched off while sign-in with
    the provider is on.") and settings locked by `OXICLOUD_OIDC_*`.
  - **New in 0.2.0** (tagged in the frames): the "Asking … for its OpenID configuration…" line and "Testing…" while
    the test runs; a failed test in plain words with the server's own words under it (0.1.0 shows
    "The server can't use this issuer URL:" and the server's words); and the guard in `-guard-untested`, which
    keeps password sign-in on until Test connection works.
  - **The sign-in sheet** (`web.rs` `login_ui`, `oidc.rs`): "Sign in with <button name>" over the rule "or" and the
    password form (secondary button); provider only (primary button, "This server signs people in through its
    identity provider only."); "Signing you in…" on the way to the provider; and each `login_error` text from
    `core::api::oidc::login_error_text` in the errors sheet, keyed as the Caddyfile and OxiCloud send them.

- **Files** (`tools/v2_files.py`, the web Files page as 0.1.0 draws it in `crates/ui-egui/src/web/files.rs`):
  - The browser: a 232 px sidebar (MY FILES, RECENT, STARRED, TRASH, and the storage meter at the bottom), the
    toolbar (breadcrumb cells, search 240, refresh, grid, NEW FOLDER, UPLOAD), the meta line (counts, or the
    selection's DOWNLOAD / STAR / MOVE / DELETE / CLEAR), then the table: 36 px header, 44 px rows, check column,
    name with the star and link marks, OPENS IN (the app's chip), MODIFIED, SIZE, "…". Selected rows have the
    accent wash and a 2 px accent rule. Types are named by format (`KIND`) where the page says them: the details
    panel ("DOCX document · 248 KB · opens in WordCraft").
  - Empty folder, loading, the "…" menu, two selected with the delete notice and its UNDO, the drop overlay
    ("DROP TO UPLOAD TO REPORTS"), the uploads panel (DONE / sizes / NAME TAKEN / WAITING), and the "This name is
    taken" sheet (SKIP, KEEP BOTH, REPLACE; replace keeps the old file as a version).
  - Sheets for new folder, rename (a sheet, as the app does) and move (a folder picker with breadcrumbs).
  - Trash: LOCATION, DELETED, DELETED FOR GOOD (sorted, soonest first), the 30-day meta line, EMPTY TRASH, the row
    menu (RESTORE, DELETE FOREVER), the "Delete 1 item for good?" sheet, and "“Notes.md” is already there" with
    RESTORE WITH THE NEW NAME (`Notes (restored).md`).
  - Recent (OPENED column, newest first), Starred, and search results with LOCATION (where each result lives).
  - Phone (under 600 px): the views as cells, square Upload and New folder buttons, the breadcrumbs on their own row,
    and a list of check, name and "…" only (SIZE needs 420 px). Dark and light.
  - **New in 0.2.0** (each marked with a "NEW IN 0.2.0" tag in its frame; the app doesn't do these yet): the
    details panel (preview, OPEN IN WORDCRAFT, DOWNLOAD, SHARE…, facts, the last versions); the version history
    sheet (the current file and the 5 kept in `.craftcloud/versions/`, RESTORE and DOWNLOAD per version); and in
    the "…" menu "Open in <app>", "Share…" (it opens the share sheet), "Versions…" and "Move to trash" (0.1.0 says
    "Delete").

## Colours (final values, `Theme` variables; dark / light)

| token | dark | light | note |
|---|---|---|---|
| bg | #121316 | #f2f1ee | |
| sunken | #060607 | #e9e8e4 | |
| ink / ink_strong | #f2f1ee / #ffffff | #101014 / #000000 | |
| muted / faint | ink 60 % / 38 % | ink 62 % / 42 % | muted 6.5:1 / 5.1:1. faint (3.3:1 / 2.7:1) fails AA, so v2 uses it for no text; only the "not connected" dot |
| line / line_strong | ink 15 % / 40 % | ink 16 % / 40 % | |
| **accent** | **#5e7a48** | **#5e7a48** | the logo green, shared with the code; surfaces: sheet rule, hover rules, progress, meters (3.9:1 on dark bg, 4.3:1 on light) |
| **accent_ink** | **#a9c48e** (9.7:1 on bg, 10.6:1 on sunken) | **#3b4d2c** (8.1:1 on bg, 7.5:1 on sunken) | text, links, headline accent word, focus ring |
| accent_tint | accent at 9 % | accent at 7 % | sheet hero wash (sheet.rs:31) |
| on_accent | #ffffff (4.8:1 on accent) | #ffffff | |
| invert_bg / invert_fg | #ffffff / #0b0b0c | #101014 / #f2f1ee | primary buttons, selected cells |
| good / warn / bad | #3fd6b8 / #ffc14d / #ff6259 | #0a7563 / #8a5800 / #b3261e | unchanged |
| on_bad | #0b0b0c | #ffffff | white on #ff6259 is 2.9:1 |
| shadow | black α140 | black α36 | 0 8 24, menus and sheets only |

`good` (#3fd6b8, hue 168°) and `accent_ink` (#a9c48e, hue 90°, much less saturated) stay distinct; `good` is unchanged.
The accent values live in `tools/lib.py` (`THEME`) and must match the code and the logo.

**Colour themes** (not in the FigCraft file): besides these values, the desktop Settings › Theme and the web
account menu offer seven craft-libs themes (Catppuccin, Dracula, Gruvbox, Nord, One Dark, Solarized Light, Tokyo
Night). Only the colours change; shapes, type and spacing stay. Their tokens come from craft-theme and are mapped
in `crates/ui-egui/src/themes.rs` (`map`: `bg` is the theme's `surface.panel`, `sunken` its `surface.sunken`, the
accent the theme's own); the shots are `activity-theme-*`, `settings-theme-catppuccin`, `web-files-theme-*` and
`web-account-menu-theme-gruvbox`.

## App names (`Apps` string variables)

Files are named by their format, never by another vendor's product: a `.pptx` is a "PPTX presentation", a `.psd` a
"PSD image", and the app that opens it is said beside it ("opens in DeckCraft"). Every text that names a Craft app
takes the name from `tools/lib.py` (`APP`), never typed again. The file also holds them as string
variables in the `Apps` collection, `<app>/name`, `<app>/blurb` (the card text) and `<app>/kind` (a file's type),
bound to the app cards' Blurb and the file rows' Type properties, so editing a variable changes every screen.
Texts that mix a name into a longer line (the details panel's "DOCX document · opens in WordCraft") are built
from the same values by the scripts, as FigCraft can't bind part of a text (limitation 9 below).

## Brand

- **`Brand/Logo` (64 px)** is the mackerel logo, `assets/logo/craftcloud-logo-256.png` as an image fill (already clipped
  to its rounded square). Instances are scaled from it; in egui, load the size-specific PNG
  (`craftcloud-logo-{16,32,48,64,128,256,512,1024,1600}.png`) nearest to the drawn size times the pixel density.
- **`Brand/Lockup`:** the logo at 24 px, a 10 px gap, then the placeholder text wordmark (`Brand/CraftCloud wordmark`,
  Archivo 620 at width 118, 18 px, "Craft" in ink_strong and "Cloud" in accent_ink).
- **Where the logo appears:** every header, the sheet heroes (44 px), setup step 1 (128 px), the tray icon states
  (22 px plus a 10 px status dot with a 1.5 px bg ring) and the window icon (48, 32 and 16 px).
- The ArtCraft mark and name are never used in branding. The footer keeps "Look based on ArtCraft Launcher by the
  ArtCraft team".

## egui implementation notes (sizes in logical px)

The launcher's own code is the reference. Each row names the function to copy (`storytold/craft-launcher@6a6f6dc crates/ui-egui/src/`).

| Component | Spec | Launcher source |
|---|---|---|
| Header | 48 tall, 1 px `line` under it. Cells fill the height and are split by vertical hairlines. **Brand cell** padding 0 20, lockup (24 logo, gap 10, wordmark 18). **Suite cell** padding 0 16, HUD "CRAFT APP SUITE" muted (web only). **Nav cell** padding 0 12, items 4 apart. Spacer. Right cells padding 0 16 with a left hairline: user (26 px avatar: sunken, line_strong border, Badge 10 initials; name Body 13), SIGN OUT (HUD muted), gear 48×48. Desktop: status cell (8 px dot, gap 8, Body 13) and window buttons 46×48 (10 px glyphs, 1 px strokes; close hovers #c42b1c) | views.rs:167 `header`, widgets.rs:135 `rule`, widgets.rs:143 `nav_icon`, widgets.rs:201 `window_buttons` |
| Nav item | 24 tall, padding 0 8, HUD (Geist Mono 500, 11, caps, 0.15 em). On: invert_bg / invert_fg. Off: transparent / muted. (FILES had a "SOON" tag before 0.1.0 shipped Files; it's gone.) | widgets.rs:577 `paint_cell` (the same painter, at 24 tall) |
| Toolbar row | 24 under the header (y 72), 34 tall, items 8 apart. Refresh square 34×34 in a hairline box; cells box; spacer; search 200×34 (sunken, hairline, padding 0 10, 16 px icon); "SORT" HUD muted plus picker 150×34; primary on the right | views.rs:292 `toolbar_row`, bar.rs:66 `refresh_button`, bar.rs:13 `view_toggle`, views.rs:316 search, views.rs:365 `sort_picker` |
| Toggle cell | 34 tall, padding 0 12, label HUD 11 (plus an optional count 8 px after it). On: invert_bg / invert_fg. Hover: sunken / ink_strong. Off: muted. One hairline box with 1 px rules between cells | widgets.rs:543 `cells_apart`, :577 `paint_cell` |
| Button | 34 tall, padding 0 16, Button 11 (Geist Mono 700, caps, 0.12 em). Primary: invert_bg / invert_fg (80 % on hover). Secondary: bg + 1 px line_strong, inverts on hover. Danger: bad / on_bad. Disabled: 50 %. Focus: 2 px accent_ink ring outside | widgets.rs:405 `button`, :410 `labelled_button` |
| Icon button | As Button, padding 0 16 0 14, a 14 px icon (1.5 px stroke in the label colour), gap 8. Split: main part, 1 px seam, then a 34×34 part with a 10 px chevron | widgets.rs:504 `split_button` |
| Status badge | 20 tall, padding 0 6, Badge 10 (Geist Mono 500, caps, 0.1 em), sunken fill, 1 px line, text in its tone. UP good, DOWN accent_ink, CONFLICT warn, DELETED bad, roles OWNER accent_ink / ADMIN good / USER muted | widgets.rs:368 `badge` |
| Footer | 34 tall, hairline on top, padding 0 24 (web) / 0 20 (desktop). Left: HUD muted status. Right: Small 12 muted links | views.rs:431 `footer` |
| Headline (hero) | Archivo 620 at width 118, 56/60 web, 40/44 desktop, −2 % tracking, ink_strong, two lines, one word in accent_ink. Block at (24, 72); 16 to the body (web), 12 (desktop) | widgets.rs:887 `heading` |
| Body | Inter 400, 18/28 muted (web hero); 14/20 elsewhere; Small 12 / 12.5 for notes | theme.rs |
| Meta line | HUD 11 muted, parts separated by "·" with 10 px gaps; link parts underlined | widgets.rs:74 `hud` |
| Sheet | Frame: bg, 1 px line_strong, shadow 0 8 24. Hero: accent_tint, padding 18 24, 44 icon, gap 12, title Archivo 26, caption HUD muted 4 under, 2 px accent rule on top, hairline under. Body padding 24, gap 16. Actions row padding 14 24, hairline on top, secondary left / primary right. Large sheets: window − 64 × window − 48 | sheet.rs:15 `size`, :21 `frame`, :27 `hero` |
| Panel | Hairline box, inner margin 14 × 12, HUD muted title, 8 to the body, 14 between panels | sheet.rs:93 `panel` |
| Settings row | 34 tall: label Body 14 left, control in a fixed-width slot at the right | settings_dialog.rs:18 `row` |
| Table | Frame bg + hairline, inner margin 0 16. Header 42 (HUD muted, sorted column ink + sort arrow). Rows 62 with a full-width hairline. Columns: remainder ≥ 220, 176, 150, 132, 104, 206 (actions). Name cell: 38 avatar, gap 10, name Inter 500 16, email Small 12 muted | views.rs:699 `table`, :792 `table_header`, :821 `table_rows`, :862 `table_app_cell` |
| Activity list | Hairline frame, padding 0 16, rows 40, items 14 apart: time Mono 12 muted, badge fixed 84, path Body 13 (fills), extra Small 12 muted right | the table pattern, single column |
| Folder table | As the table: header 36, rows 40 (34 inside the onboarding sheet), columns fill / 90 files / 90 size / 112 Off-On cells | views.rs:699 |
| App card | Grid columns from 320 px, cards share 1 px hairlines. Strip 40 (version HUD in app ink, state HUD right: "IN THE BROWSER" good / "OFF" muted). Body padding 18 20 20 20, gap 14: 56 icon with shadow (0 6 14, α71, 12 radius on the shadow only), 22 two-tone wordmark, 13.5 tagline, meta row (EXPERIMENTAL badge, OPENS HUD muted), Off/On cells at the bottom. Hover: 5 % app-colour wash, 2 px top rule in the app colour, icon up 4. **Height 252** (launcher 314: no buttons or blurb here) | views.rs:532 `grid_block`, :554 `card` |
| Card off state | Icon greyscale, the content block at 55 % with a 2.5 px blur, wordmark "Craft" muted, strip version muted. **egui has no blur:** at build time, ship a pre-blurred greyscale copy of each icon, and draw the text at 55 % without blur. The cards here blur the text too; a faded, unblurred text is the egui version | `icons::app_image(.., in_colour=false)` |
| Progress | Caption Small 12 above, 4 px track (line), fill accent | widgets.rs:613 `progress` |
| Details overlay | Card window − 64 × window − 40 over the scrim. Hero with accent_tint and rule, 64 avatar, kicker HUD, name Archivo 32, tags; actions right; two columns of panels | details.rs:93 `details_drawer`, :312 `details_hero` |
| Menu | bg, 1 px line, shadow; items 32 tall, padding 0 12, Body 13; hover sunken | widgets.rs:796 `more_menu` |
| Code | Geist Mono 500, 28, 8 % tracking, in a 72 tall sunken hairline box | — |
| Tray | Logo 22 plus a 10 px dot (good / accent_ink / warn / bad) with a 1.5 px bg ring; menu 250 wide, Recent files submenu 200 (the OS draws the real menu) | `apps/craftcloud/src/tray.rs` |

**Spacing:** PAD 24 page margin; 14 between panels; 8 between toolbar items; 16 between form fields. Every corner is
square except app icons and the logo.

## How it was made

- `tools/build.py <figcraft-cli>` drives the headless `figcraft-cli mcp` (`tools/fc.py`).
- The build runs with the x86_64 `figcraft-cli` 0.18.1, its sha256 checked against the release's
  `SHA256SUMS.txt` and the GitHub asset digest.
- It builds the tokens, the component library, the old frames (then archived) and the v2 frames (`tools/v2.py`), then
  saves and exports.
- A full run takes about 14 s.

```sh
nice -n 10 python3 -I design/tools/build.py /path/to/figcraft-0.18.1-linux-x86_64/bin/figcraft-cli
```

**Checked in the web build:** `tools/web_verify.py` opened the saved file in the unmodified FigCraft web build in
headless Chromium. It found 5 pages, 16,810 layers, no repairs, and `v2-desktop-activity` rendered: see
`verification/web-v2-desktop-activity-fonts1.png`.

## Notes and deviations

- **Faint text:** muted wherever faint would fail AA, so v2 draws no text in `faint`: meta lines,
  HUD captions and footers are muted (6.5:1 dark, 5.1:1 light).
- **The getartcraft sub-strip** (`01 / SECTION · facts`) isn't a separate bar: the launcher wins. The kicker "01 / HOME"
  sits above the headline, and facts go in meta lines and in the admin facts strip.
- **CTA cell:** the website's flush-right "LAUNCH APP" cell is replaced by the launcher's gear cell. On the desktop,
  SETTINGS is a header cell that opens the settings sheet, so there is no gear there.
- **Danger buttons** in dark mode keep near-black text (`on_bad`), because white on #ff6259 is 2.9:1.
- **A third demo user**, "Test Account" (deactivated), shows the DEACTIVATED and ACTIVATE states.

## FigCraft 0.18.1 limitations and bugs found (possible upstream issues)

1. **`style.create {type: "text", value}` silently drops `fontVariations` and `textCase`.** The style keeps Regular
   weight and original case, with no error, although unknown parameters are errors elsewhere. Workaround: `style.create {from: <text layer>}` keeps both.
2. **`style.bind {variable: "Theme/shadow"}` fails with "no such variable".** `$var` accepts the same `Collection/name`
   form. The plain name `shadow` works.
3. **`ui.screenshot` / `figcraft.screenshot()` times out in headless Chromium** with "did not answer ui.screenshot within 30 s
   (a hidden tab …)". Every other control call answers. The CDP page screenshot works.
4. **Setting `absolute: true` on an auto layout child keeps its position in the flow**, so `x`/`y` must follow in a second `node.set`.
   This may be intended; it is easy to trip over.
5. **The web build has no glyph fallback for some symbols the desktop build draws:** ▾ ▴ ✕ come out as tofu. I replaced them with vector icons.
6. **A `.figcraft` file doesn't carry its fonts.** Without Archivo and Geist Mono installed (or loaded with `text.loadFont` or Local
   Font Access in the web build), wordmarks and labels fall back. `figcraft-cli info` lists 273 + 1164 text layers with missing
   fonts. Inter is built in.
7. **`node.find {name}` matches substrings, and a nested instance can share its name with its parent cell** (the
   header's "Search" cell and the "Search" field). Exact names have to be filtered client-side.
8. **A multi-layer PNG export** (the overview sheets) has a transparent background, not the page colour.

9. **Text ranges can't bind variables** (`text.style {textFills: [{"$var": …}]}` is refused). The headline
   accent word is therefore a literal per mode (#a9c48e / #3b4d2c), and these headlines don't follow a theme switch.

## Credits

- **App icons:** `storytold/craft-launcher` `assets/app-icons/` (MIT OR Apache-2.0). `artcraft.png` and `craft-anything.png` are not used.
- **Fonts:** Inter, Archivo, Geist Mono (SIL OFL 1.1; the licences are in `assets/fonts/`).
- **WordCraft screenshot:** `storytold/craft-launcher-assets` `wordcraft/hero.jpg` (WordCraft UI, MIT OR Apache-2.0; the sample document is original text written for WordCraft, per that repo's `ATTRIBUTION.md`).
- **The look** is based on ArtCraft Launcher by the ArtCraft team. No ArtCraft logo, mark or name is used in CraftCloud's branding.
