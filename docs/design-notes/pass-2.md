# UI pass 2

The second pass over the web frontend, after [pass 1](pass-1.md). Each change says what changed
and why. The screenshots come from the kittest renderer, as in pass 1:

```sh
CRAFTCLOUD_SHOTS=/some/dir cargo test -p craftcloud-ui-egui --features shots -- --ignored web_shots
```

## App cards (setup step 4, Admin › Server)

- **The v2 card.** `app_grid.rs` now draws the card from `design/README.md` and
  `v2-web-admin-server`:
  - 252 px tall, columns from 320 px, cards sharing 1 px rules;
  - a 40 px strip with the version in the app's ink and IN THE BROWSER (good) or OFF (muted);
  - the 56 px icon with its shadow, the two-tone wordmark ("Word" ink, "Craft" in the app's
    ink) and a one-line tagline cut with "…";
  - the meta row (EXPERIMENTAL badge, OPENS .DOCX .ODT .MD …) and Off/On cells at the bottom.

  Before, it was the compact card from pass 1, with a colour chip in place of the icon.
- **Off.** The icon turns grey and the content is drawn at 55 %. egui has no blur, so the card
  is faded but not blurred, as the README's egui note allows.
- **Hover.** A 5 % wash in the app's colour, a 2 px top rule, and the icon lifts by 4 px.
- **One control per card.** The whole card stays one checkbox named after the app, and a click
  anywhere toggles it. The Off/On cells are only drawn, so the page's own OFF/ON cells stay the
  only widgets with those labels.
- **Icons.** The image's `/craftcloud/` has no icons (only `apps.json`), so the web build can't
  fetch them. The 15 Craft app icons (128 px, about 220 KB in all) are built in, from
  `crates/ui-egui/assets/app-icons/`, which are copies of `design/assets/icons`. They come from
  craft-launcher's `assets/app-icons/`, MIT OR Apache-2.0, and are credited in `NOTICE`. An app
  with no icon gets a rounded tile in its colour with its initial.
- **The grid fills its row.** Column edges are rounded, so the last card ends on the page margin
  (before, a few pixels were left over).

## Wrapping on a phone

- **Meta lines wrap between their parts.** Each "· part" asks for its measured width, so
  `horizontal_wrapped` moves it to the next line. Before, the line ran off the screen and
  pushed the whole sheet body wider. This happened on setup step 4 on a phone ("-APPS IMAGE ·
  ABOUT 770 MB DOWNLOAD · 2.1 GB ON DISK"), and the app cards then overflowed the sheet.
- **Sheet captions are meta lines,** so "STEP 4 OF 5 · WEB APPS · OPTIONAL" wraps instead of
  running past the sheet.
- **Step 4 on a phone** uses the 12 px sheet margin like every other sheet, not window − 64, so
  the cards get the room.
- **Home on a phone.** The paragraph's `set_max_width(640)` widened the column past a 390 px
  screen: egui's `set_max_width` grows a narrower ui. The paragraph, meta line and panels ran
  off the right edge. It is now capped at the page width.

## Admin

- **Users on a phone.** Each user card is 146 px tall: the name and email, then the role and
  status badges with the storage, then RESET PASSWORD (or ACTIVATE) and "…" on a line of their
  own. Before, the cards were 92 px, the "…" shared a line with the name, and the rules crossed
  the avatars.
- **Server on a phone.** SAVE and the app search get a row under the switches, with the search
  filling it. Before, SAVE was laid over "6 of 15 on".
- **Spacing.** The meta line (Users) and the facts strip (Server) sit 16 px under the toolbar,
  as Files' meta line does (it was 24).

## Files

- **Trash, DELETED column (T-048).** Two items of the same name were told apart only by their
  purge dates. The trash now has a DELETED column with the date and the time, in local time
  ("Yesterday 22:54", "2 Nov 2026 09:41"), and it sorts.
  - It shows at every width. On a phone it takes the size's place.
  - DELETED FOR GOOD keeps its own column from 760 px. Narrower, the row's tooltip says it.
  - `time::when_exact` keeps the time on older dates. `when` still drops it in other lists.
- **Trash meta on a phone:** "kept 30 days" instead of "deleted for good after 30 days", which
  didn't fit on the one line.
- **Upload panel** sits on the page margin (24, a phone 16), not 16 from the edge.
- **Recent (T-017) checked:** the header shows OPENED ▼ and the list is newest first.

## Kept versions in the web frontend (T-049, T-050)

These were not in the pass's brief; they were added on request during it.

- **A rename or a move** sends `versions::follow_move` for each item once it has worked:
  `MKCOL` for the parents, then `MOVE` for the history.
- **An upload that makes a new file** drops the history still kept at its path, because that
  history belongs to the file that was there before. The CLI's `put` does the same after it
  saves.
- **Delete forever and Empty trash** drop the histories of what went for good. A `PROPFIND`
  checks the path first, and the history stays if something is there again.
- **How the steps run** (`web/files/history.rs`): one request at a time, only after the change
  itself worked, and best effort. A failed step neither undoes the change nor stops the next
  step.
- **Restored names:** the web restore names "… (restored)" with core's `names::restored_name`,
  as the CLI does.

## Share page (T-015)

- **Subfolders open in place.** A shared folder's subfolders are rows that open with OPEN. The
  link's cookie lets a visitor list them.
- **Breadcrumbs** above the list lead back up as far as the shared folder, never above it.
  Names keep their case.
- **DOWNLOAD ALL** still zips the whole shared folder.

## Smaller fixes

- **Hero paragraphs** have the design's 18/28 line height. The smaller body sizes use about
  1.43. Before, egui's default line height applied, about 22 for size 18.
- **Home's device line** says "Connected 13 Nov 2026". It lower-cased the whole date ("13 nov
  2026"). Now only "today" and "yesterday" are lower case.

## Shots added to `web::shots`

- **New scenes:**
  - Files: `files-upload` (the panel mid-upload), `files-upload-taken` (the taken-name sheet),
    `files-new-folder`, `files-rename`, `files-move`, `files-menu` (the row's "…" menu).
  - Device: `device-connected` and `device-denied`.
  - Share: `share-subfolder`.
- **Changed data:**
  - `files-trash` has two "notes.md" items, so T-048 shows.
  - The Files scenes have a wall clock (UTC+1).
  - The app catalog in the shots lists all 15 apps from `apps.toml`.

## Compared, not changed

- **Setup 1 and 3.** Setup 1 follows its frame apart from the line height (fixed above).
  Setup 3 shows the address facts, not the frame's editable PUBLIC ADDRESS field with CHECK
  and the storage facts. That is a feature gap, not a styling one.
- **Device Connected.** The frame has a "TODAY 14:31" HUD beside the CONNECTED badge. The page
  doesn't keep when the approval happened, so it is left out.
- **The rename, move and new-folder sheets** follow the sheet spec. The move picker's folder
  rows have no chevron. That was left as it is.
- **Desktop.** The shots of every scene (light, and 640×480) show nothing clipped or
  overlapping, and nothing changed there in this pass. At 640×480 the Settings sheet's body
  scrolls, which is fine. Its THIS DEVICE field label is in ink while the facts' keys above it
  are muted: the same field style as everywhere else, left as it is.
- **The share sheet.** The web file browser has no "create a share link" sheet yet: no menu
  item, no `Sheet` variant. Only the public share page exists, so there is nothing to compare.
