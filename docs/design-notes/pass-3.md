# UI pass 3: other servers on the desktop

Frames only, for approval before any code: what the desktop client shows on a server that isn't
CraftCloud's own ([WEBDAV-STORAGE.md](../WEBDAV-STORAGE.md) § 10 and § 11). They are on the page
**v2 · Servers** of `design/craftcloud.figcraft`, built by `design/tools/v2_servers.py`, each dark
and light (`design/exports/<name>.png` and `<name>-light.png`, 2x in `exports/2x/`). No existing
frame changed.

## What was added

| Frame | What it shows |
|---|---|
| `v2-desktop-connect-found` | The Connect sheet with the address field as today, "Nextcloud 31 at cloud.example.org", the way on (Sign in in the browser) and the link "Another kind of server" |
| `v2-desktop-connect-kind` | The picker: CraftCloud, Nextcloud, ownCloud, OpenCloud, Other WebDAV server, each with how one signs in; the detected kind is marked FOUND |
| `v2-desktop-connect-webdav` | Other WebDAV server: the WebDAV address, the user name, "an app or WebDAV password, not your account password", and the plain-http note |
| `v2-desktop-connect-app-password` | ownCloud: the user name and an app password made in Settings › Security |
| `v2-desktop-connect-app-token` | OpenCloud: the user name and an app token made in Account › App Tokens |
| `v2-desktop-check-download-only` | After the check: the Download-only panel, What was checked, Check again, and the way on |
| `v2-desktop-check-not-possible` | After the check: the Not-possible panel, What was checked, Check again |
| `v2-desktop-activity-download-only` | The window with the "Download only" badge beside the state in the header |
| `v2-desktop-tray-download-only` | The tray menu's status line with "· Download only", and the header's status cell in each state |
| `v2-desktop-settings-server` | Settings with the Server panel: kind, address, mode, last check, What was checked, Check again, Open in the browser |
| `v2-desktop-settings-checked` | "What was checked": the 18 checks in plain words, as a Nextcloud 31 answers them |

## Choices made

- **The list of checks is its own sheet.** Eighteen rows don't fit a Settings panel in a 960 × 640
  window, so the Server panel has the facts and the buttons, and "What was checked" opens a sheet
  with a scrolling list. The same sheet opens from the two panels after the check.
- **The badge is the existing status badge, in warn.** The code today says it as text ("Up to date ·
  Download only"); the frames draw a badge after the status text in the window, and keep the words
  in the tray menu, which the OS draws.
- **The forms name the server in a box at the top** with "Another kind of server" beside it, so the
  way back to the picker is always the same link.
- **The check rows** are the strings of `crates/core/src/storage/probe.rs::checks`, with the values
  § 5 gives for Nextcloud 31. The names row shows the strict rules; it wasn't measured for this frame.
- **No vendor marks:** the other servers are named in text only.

## Open wording questions

Texts that § 10 and § 11 don't give, written for these frames:

1. The note under the found server: "You sign in on the server's own page in your browser. CraftCloud
   gets an app password made for this computer, never your account password."
2. The picker's lines: "Sign in in the browser", "An app password from Settings › Security", "An app
   token from Account › App Tokens", "The WebDAV address, a user name and a password".
3. The forms' titles ("Your WebDAV server", "Sign in to ownCloud", "Sign in to OpenCloud") and the
   lines saying where to make the secret: "Make one in ownCloud under Settings › Security and paste it
   here: an app password, not your account password." and the same for OpenCloud with "Account › App
   Tokens" and "an app token".
4. "A password is never sent over plain http, except to this computer." § 10 says "except to a
   loopback address"; "this computer" is the plain word for it.
5. The way on from Download only: "Continue with download only", with "Use another server" as the
   way back. From Not possible: "Use another server" and "Quit".
6. The step cells after the check: step 2 lit as "Check" (caption "Step 2 of 3 · the check"). The
   forms call step 2 "Sign in"; the browser flow keeps "Browser".
7. The sheet titles "Download only" and "Not possible", and how the server is named when its kind is
   plain WebDAV ("WebDAV server at cloud.example.org").
8. On the "What was checked" sheet: the column head "What the server did", and the note "The check
   writes only in a folder of its own on the server (.craftcloud/probe/) and removes it afterwards. It
   never touches your files."
9. The footer under download only: "Download only · checked for changes 2 min ago · cloud.example.org"
   ("Synced 2 min ago" would say too much).
10. Settings › Server: the mode as "Two-way" / "Download only" (capitalised), and the date as
    "18 Nov 2026 at 10:00".

## Open questions beyond wording

- **A badge or words in the header?** The code and its test say "Up to date · Download only" as one
  text. The badge is easier to see; it needs the status cell to draw one.
- **Warn for the badge.** Amber is also "needs you" (a conflict). The alternative is accent_ink, as
  the DOWN badge.
- **A refused server has no table of capabilities,** only the steps that ran and the reason. What its
  "What was checked" shows (the steps up to the one that refused) has no frame yet.
- **Settings › Server under download only** has no frame: the Mode row would carry the badge and the
  panel's sentence.
- **OpenCloud's sign-in in the docs.** § 10's table still gives OIDC "to verify" for OpenCloud, and
  § 5 writes the page as "App tokens"; the frames follow the decision that it is an app token made
  under Account › App Tokens.
- **The link only shows behind `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`** until 0.3.0 (§ 16); the frames
  show the screens with it on.
- **The WebDAV form shows no error state** (an `http://` address that isn't this computer, a wrong
  password). Those come with the build.
