# UI pass 1

A pass over every screen against the v2 frames in `design/exports/` and the launcher look. Each
change below says what changed and why. Screens with no entry were checked and left as they were.

Screenshots come from the kittest renderers (`--features shots`): `desktop::shots::shots` (every
desktop scene at 960×640 dark and light, and 640×480) and the new `web::shots::web_shots` (every web
page at 1440×900 dark and light, and 390×844, a phone):

```sh
CRAFTCLOUD_SHOTS=/some/dir cargo test -p craftcloud-ui-egui --features shots -- --ignored shots
```

## Shared parts (`chrome.rs`)

- **Meta lines** are as tall as their text, not a 34 px control, so the line under a headline sits
  on the design's 12 px gap (it was about 20). A "·" now stays with the part after it, so a
  wrapped meta line (Admin › Users on a phone) never ends in a dot.
- **Headlines** lay their lines out with no item gap, on the design's 56/60 and 40/44.
  `headline_toned` lets an error state colour its marked word (see Activity).
- **Facts panels** (device request, shared file, account details) show values in Body 14, as the
  device approval frame does, instead of mono 13. Long values wrap inside the panel.
- **Sheets narrower than 560 px** drop the step cells from the hero. On a phone they covered the
  title ("Your cloud, read…"). The caption still says "Step 5 of 5".
- **Copy row** (an address in a sunken box with COPY) moved from the web home page to `chrome`,
  so the desktop sign-in uses the same one. The address is clipped to its box.
- **Web header on a phone:** SIGN OUT was cut to "OUT". It is now the sign-out door icon with the
  same accessible label.

## Desktop

- **Sign-in, step 2 (T-020).** The sheet always shows the sign-in address with a COPY button.
  When the system browser didn't open, it says "Open this address in your browser, sign in, and
  check it shows this code:" instead of "We opened … in your browser", and the button reads TRY
  THE BROWSER AGAIN. `DesktopHost::open_browser` reports whether opening worked; the native host
  returns the result of `craftcloud_native::open_url`.
- **Activity, folder missing (T-039).** The headline's "gone" is drawn in `bad`, like the FOLDER
  MISSING badge, not in the accent green. Offline and Failed follow the same rule: an error
  state's marked word takes its badge's colour. SYNC NOW and OPEN FOLDER are disabled while the
  folder is missing, with "Off until the folder is back" beside them and the reason on hover. The
  meta line's OPEN FOLDER link is left out. LOOK AGAIN in the panel is the way on.
- **Tray menu.** "Open CraftCloud folder" and "Sync now" are disabled while the folder is missing,
  as in the window.

## Web

- **Files › Recent (T-017).** Recent lists files in the order they were opened, but the header
  showed NAME ▲. Recent now has an OPENED column (from `accessed_at`) in place of MODIFIED,
  sorted newest first, and its header carries the arrow. Clicking another column sorts by it as
  before. Leaving Recent goes back to name order.
- **Files on a phone.** The breadcrumbs overlapped the search box. On a phone the search and
  actions take the toolbar row, and the breadcrumbs get a row of their own under it. At the top
  of My files they are left out, because the view cells already say where you are. The search
  placeholder is centred in its 34 px box (it sat high).
- **Files, empty and loading.** An empty folder offers UPLOAD FILES under its note. Notes wrap and
  centre (a long error no longer runs off a phone screen). "Loading…" is a HUD label like
  everywhere else.
- **Files › Trash (T-044).** The trash comes soonest-deleted first. Its DELETED FOR GOOD header
  now carries the arrow instead of NAME ▲. The meta line counts items ("9 items · … · deleted for
  good after 30 days", or "2 of 9 items" while filtered).
- **Trash filter (T-043).** Typing in the search box in the trash used to jump to My files. The
  trash now filters its own list as you type ("Filter the trash"). The server's search doesn't
  look in the trash.
- **Search after a change (T-043).** OxiCloud caches search answers per exact query string for a
  few minutes, so a trashed or moved file stayed in the results. A file sent to the trash now
  leaves the list at once. After every change, the next search asks with a slightly different
  page size (`limit=99`, `98`, …), so the server answers afresh.
- **Restore onto a taken name (T-047).** Restoring an item whose old place is taken used to give
  a raw server error. Restore now checks the place first, over WebDAV. When it is taken, a sheet
  says "“b.txt” is already there" and offers RESTORE WITH THE NEW NAME, "b (restored).txt".
  OxiCloud can't restore under another name or rename in the trash, so this runs four steps: move
  the file that's there aside, restore, rename the restored one, then move the first file back.
  If a step fails, the first file is moved back.
- **File names keep their case (T-046).** The rename, new-folder and restore sheets show the name
  or place in the hero in its own case, not in HUD caps (`Hero::keep_case`). A breadcrumb that is
  a folder's name keeps its case too. "My files" and the view names stay HUD labels.
- **Home (T-045).** OxiCloud 0.10.0 lists no devices for some signed-in desktop clients. An empty
  list no longer shows as "0 devices": the count is left out. No other endpoint gives the number
  to a non-admin.
- **Upload input.** The browser upload now uses one hidden `<input type=file
  id=craftcloud-upload-input>` that stays in the page, with one change handler. Before, a new
  input was made for every pick and never added to the page. That worked in browsers, but a test
  driver couldn't find it (CDP's `DOM.setFileInputFiles` needs a node in the document).
- **Shared link page.** DOWNLOAD and DOWNLOAD ALL use the download icon, not the arrow. A shared
  folder's list has a HUD count ("3 items in this folder", or "This folder is empty."), hairlines
  between rows and rows a control tall.

## Compared, not changed in this pass

- **Compared with their frames:**
  - Web: setup steps 2, 4 and 5, sign-in, device code entry and approval, home, Files (browse,
    Recent, empty), the shared-file page, Admin › Users (phone) and Admin › Server.
  - Desktop: browser sign-in, Activity, the states, Folders, Settings and Conflict (640×480).
  - Apart from the fixes above, they follow their frames.
- **Admin › Server and setup step 4: the app cards** are a compact card, without the v2 card's
  version strip, 56 px icon or Off/On cells. Bringing them to the v2 card (`v2-web-admin-server`,
  launcher `views.rs:554`) is the largest gap left, for pass 2.
- **Admin › Users on a phone:** the user cards are tight but readable. More air is for pass 2.
- **Not looked at in this pass:**
  - Web: setup steps 1 and 3, device Connected and Denied, the upload, rename, move and share
    sheets. The sheets need a scripted flow to render; the kittests cover how they behave.
  - Desktop: the tray menu (a native menu, not egui).
