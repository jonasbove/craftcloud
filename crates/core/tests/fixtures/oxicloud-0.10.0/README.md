Recorded against craftcloud:dev (OxiCloud 0.10.0) on 2026-11-13; tokens and device codes redacted.
User john / john@example.org is a throwaway test account on a throwaway container.

The file API answers (folders-*, folder-*, file-*, files-in-folder, batch-*, trash*, favorite*, recent*, search*,
share*) were recorded against craftcloud:dev-apps on 127.0.0.1:8095, in this order: create Documents and Reports,
upload plan.txt, rename it to "plan v2.txt" and move it to Reports, create Reports/Q3, rename it to Q4 and move it
into Documents, star, record recent, search, share, batch copy/move/trash, trash and restore. Share links there
are to that throwaway server and no longer work. openapi.json is the server's own spec.

Admin and OIDC answers (`admin-user*.json`, `admin-users-three.json`, `admin-sessions.json`, `oidc-*.json`) were
recorded against craftcloud:dev-apps the same way: users jane and test are throwaway demo accounts, and the OIDC
settings point at the placeholder issuer `https://id.example.org` (never contacted: `oidc-providers-on.json` is
what the server reports once those settings are saved and enabled).

search-with-versions.json: searching "Notes" on a server where Documents/Notes.md has one CraftCloud version
(recorded against craftcloud:web-files, 2026-11-14): the version folder and the version are found too.
