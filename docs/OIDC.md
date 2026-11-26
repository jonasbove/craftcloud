# Single sign-on (OpenID Connect)

CraftCloud can let people sign in with an OpenID Connect (OIDC) identity provider. It uses OxiCloud's own OIDC
support, unmodified: CraftCloud's web frontend shows the **Sign in with …** button and finishes the sign-in, and
**Admin › Server › Single sign-on** edits the settings. Nothing in CraftCloud is tied to one provider.

The examples use `https://cloud.example.com` for CraftCloud and `https://id.example.com` for the identity provider.
Replace both with your own addresses.

## How the sign-in works

1. The sign-in page asks the server whether a provider is set up (`GET /api/auth/oidc/providers`) and shows
   **Sign in with <name>**, where the name is the **Button name** setting (empty: "single sign-on").
2. The button opens `/api/auth/oidc/authorize`. OxiCloud reads the provider's metadata from
   `<issuer URL>/.well-known/openid-configuration` and redirects to the provider's authorization endpoint: the
   authorization code flow with PKCE (S256), a random `state` and a `nonce`.
3. After the person signs in there, the provider sends the browser to the **redirect URI**,
   `https://cloud.example.com/api/auth/oidc/callback`. OxiCloud checks the state, swaps the code for tokens
   (sending the client secret in the request body, `client_secret_post`), checks the ID token's signature (RS256,
   RS384, RS512, ES256 or ES384 from the provider's JWKS), issuer, audience and nonce, and redirects to
   `https://cloud.example.com/login?oidc_code=…`.
4. CraftCloud's frontend swaps that one-time code (valid for 60 seconds, usable once) for a session
   (`POST /api/auth/oidc/exchange`) and opens the home page.

Every step goes through CraftCloud's own address: Caddy passes `/api/*` to OxiCloud and serves `/login` from the
frontend, so nothing extra has to be opened up. Both addresses come from `CRAFTCLOUD_PUBLIC_URL`; set it to the
address people type, with `https://`, before you register the client.

The issuer URL must be reachable **from inside the CraftCloud container** (metadata, keys, token exchange) **and
from people's browsers** (the sign-in page), under the same name: the ID token's issuer has to match the one in
the metadata. A provider on the same Docker host therefore needs a public name (or one that resolves the same way
inside the container), not `localhost`.

## Who gets in, and as whom

When someone signs in through the provider, OxiCloud looks for the account in this order:

1. **Already linked:** an account that signed in with this provider before (matched by the ID token's `iss` and
   `sub`).
2. **Same email:** exactly one account with the same email address (case and `+tag` ignored) gets linked to the
   provider, if the provider says the address is verified (`email_verified: true`). This is
   `OXICLOUD_OIDC_AUTO_LINK_EMAIL_MATCH`, on by default and only settable in `.env`.
3. **New account:** with **Create accounts at first sign-in** on (the default), OxiCloud creates one:
   - **username:** the `preferred_username` claim, else `name`, else `oidc_<sub>`. The part before an `@` is kept;
     only `a-z 0-9 - _ .` stay, lowercased, at most 32 characters. If it is taken, a short suffix from `sub` is
     added.
   - **email:** the `email` claim (if the ID token has none, OxiCloud asks the userinfo endpoint).
   - **role:** admin if one of the `groups` claim's values is in **Admin groups** (compared without case),
     otherwise user. An identity provider never makes anyone the owner.
   - With it off, people without an account are refused (the sign-in page says so).

**Admin groups are applied when an account is created.** To change an existing person's role, use
**Admin › Users**. The claim must
be called `groups` and hold a list of names.

An account that signs in through the provider has no password; people with a password can still use it, unless
password sign-in is off.

## 1. Register CraftCloud with the provider

Create an OIDC client (often called an "application" or "OAuth2/OpenID provider"):

| Setting | Value |
|---|---|
| Client type | Confidential (with a client secret), authorization code flow |
| Redirect URI (callback URL) | `https://cloud.example.com/api/auth/oidc/callback` (exactly) |
| Scopes | `openid profile email`, plus `groups` if you use admin groups |
| PKCE | Allowed or required, S256 (OxiCloud always sends it) |
| Token endpoint authentication | `client_secret_post` (the secret in the request body) |
| ID token signing | RS256 or ES256 (not HS256, not EdDSA) |
| Sign-out / logout URI | Not needed |

Admin › Server shows the exact redirect URI for your server under **Register with your provider**. Then note the
provider's **issuer URL**, the **client ID** and the **client secret**.

## 2. Turn it on in CraftCloud

**In the browser (recommended):** sign in as the owner or an admin, open **Admin › Server** and scroll to
**Single sign-on**:

| Field | What to enter | Same as |
|---|---|---|
| Button name | What the button says after "Sign in with", e.g. the provider's name | `OXICLOUD_OIDC_PROVIDER_NAME` |
| Issuer URL | From step 1 | `OXICLOUD_OIDC_ISSUER_URL` |
| Client ID | From step 1 | `OXICLOUD_OIDC_CLIENT_ID` |
| Client secret | From step 1. Stored on the server and never shown again; type a new one to replace it | `OXICLOUD_OIDC_CLIENT_SECRET` |
| Scopes | `openid profile email`, plus `groups` for admin groups | `OXICLOUD_OIDC_SCOPES` |
| Admin groups | Optional, comma-separated: members become admins when their account is created | `OXICLOUD_OIDC_ADMIN_GROUPS` |
| Sign-in with the provider | On | `OXICLOUD_OIDC_ENABLED` |
| Create accounts at first sign-in | On: anyone the provider lets in gets an account (with the default quota). Off: only people who already have one | `OXICLOUD_OIDC_AUTO_PROVISION` |
| Password sign-in | On: the password form stays next to the button. Off: single sign-on only (only while sign-in with the provider is on, and after **TEST CONNECTION** worked) | `OXICLOUD_OIDC_DISABLE_PASSWORD_LOGIN` (inverted) |

**TEST CONNECTION** asks the server to read `<issuer URL>/.well-known/openid-configuration` ("Asking
id.example.org for its OpenID configuration…" while it runs) and shows what it got, so you see a wrong address
or an untrusted certificate before anyone signs in. A failure is put in plain words (the name doesn't resolve
from inside the container, nothing answers, no answer in time, an untrusted certificate, no OpenID configuration
at that address), with the server's own words under it ("Server said: …"). The plain words are picked by matching OxiCloud
0.10.0's English error text (`test_failure_text` in `crates/core/src/api/oidc.rs`); if a later OxiCloud words it
differently, the page falls back to a general line, and the server's own words still show.

Password sign-in goes off only after **TEST CONNECTION** worked for the issuer URL in the form, in the same visit
to the page: with the provider out of reach, nobody could sign in. The same holds when password sign-in is already
off and the issuer URL changes. Otherwise the page answers "Password sign-in stays on until Test connection
works" and saves nothing. This guard is in the page only: a direct call to the server's
`PUT /api/admin/settings/oidc` still turns password sign-in off. It keeps an admin from locking everyone out by
mistake; it is not a security boundary. **SAVE SINGLE SIGN-ON**
applies the settings at once; no restart is needed. Sign out and the sign-in page shows the new button.

**In `.env` instead:** the compose file passes every `OXICLOUD_*` line in `.env` through (see `.env.example`).
Values set this way win over the admin page, which then shows them as read-only. Restart the container after
changing `.env`.

```sh
OXICLOUD_OIDC_ENABLED=true
OXICLOUD_OIDC_ISSUER_URL=https://id.example.com
OXICLOUD_OIDC_CLIENT_ID=craftcloud
OXICLOUD_OIDC_CLIENT_SECRET=<the client secret>
OXICLOUD_OIDC_SCOPES="openid profile email groups"
OXICLOUD_OIDC_PROVIDER_NAME="Example ID"
OXICLOUD_OIDC_AUTO_PROVISION=true
OXICLOUD_OIDC_ADMIN_GROUPS=craftcloud-admins
```

The redirect URI and the page OxiCloud sends people back to both follow `CRAFTCLOUD_PUBLIC_URL` (CraftCloud sets
`OXICLOUD_BASE_URL` from it). `OXICLOUD_OIDC_REDIRECT_URI` and `OXICLOUD_OIDC_FRONTEND_URL` override them; you
only need them if the server is reached under another address than its public one.

### Single sign-on only

Switch **Password sign-in** off on the admin page (or set `OXICLOUD_OIDC_DISABLE_PASSWORD_LOGIN=true`, or
`OXICLOUD_AUTH_METHODS=oidc`, in `.env`). The sign-in page then shows only the provider's button, and
`POST /api/auth/login` is refused. With `OXICLOUD_AUTH_POLICIES=auto_redirect_if_standalone_oidc` in `.env` as well,
the sign-in page goes straight to the provider (but not after an error, so it can't loop).

If the provider breaks while password sign-in is off, set `OXICLOUD_OIDC_DISABLE_PASSWORD_LOGIN=false` in `.env`
and restart: the environment wins over the admin page, and passwords work again.

## Provider recipes

Each recipe gives what CraftCloud needs; the provider's own documentation has the clicks. Only **Keycloak** was
tested end to end with CraftCloud (see the end of this page); the others follow the providers' documentation and
are untested.

### Keycloak (tested)

- **Client:** Clients › Create client, type OpenID Connect, **Client authentication** on, **Standard flow** on.
  Valid redirect URIs: `https://cloud.example.com/api/auth/oidc/callback`. The secret is under Credentials.
- **Issuer URL:** `https://id.example.com/realms/<realm>`.
- **Claims:** `preferred_username` (the Keycloak username), `email`, `email_verified` (the user's "Email verified").
- **Groups:** Client scopes › Create client scope `groups` (type OpenID Connect, include in token scope), add a
  **Group Membership** mapper with token claim name `groups` and **Full group path off**, then add the scope to the
  client (Default or Optional). Scopes in CraftCloud: `openid profile email groups`.

### Authentik (untested)

- **Client:** Applications › Providers › **OAuth2/OpenID Provider**, client type **Confidential**, redirect URI
  `https://cloud.example.com/api/auth/oidc/callback` (strict), and pick a **signing key** (an RS256 certificate;
  without one Authentik signs with HS256, which OxiCloud doesn't accept). Create an application for it.
- **Issuer URL:** `https://id.example.com/application/o/<application-slug>/`.
- **Claims:** `preferred_username` (Authentik username), `email`, `email_verified`.
- **Groups:** Authentik's default `profile` scope mapping already adds a `groups` claim with the group names;
  scopes `openid profile email` are enough.

### Authelia (untested)

- **Client:** in `identity_providers.oidc.clients`: `client_id: craftcloud`, a hashed `client_secret`,
  `public: false`, `redirect_uris: [https://cloud.example.com/api/auth/oidc/callback]`,
  `scopes: [openid, profile, email, groups]`, `require_pkce: true`, `pkce_challenge_method: S256`,
  `token_endpoint_auth_method: client_secret_post`, `authorization_policy` as you like.
- **Issuer URL:** Authelia's own address, e.g. `https://auth.example.com`.
- **Claims:** `preferred_username`, `email`, `email_verified`, `groups` (with the `groups` scope). Recent Authelia
  versions put these in the userinfo answer rather than the ID token; OxiCloud reads userinfo only when the ID
  token has no `email`, which is then the case, so it works. If you add `email` to the ID token with a claims
  policy, add `groups` and `preferred_username` too.

### Pocket ID (untested)

- **Client:** OIDC Clients › Add, callback URL `https://cloud.example.com/api/auth/oidc/callback`, not a public
  client (it gets a secret). PKCE may be on.
- **Issuer URL:** Pocket ID's address, e.g. `https://id.example.com`.
- **Claims:** `preferred_username`, `email`, `email_verified`.
- **Groups:** Pocket ID's user groups come as a `groups` claim with the `groups` scope: scopes
  `openid profile email groups`. Optionally allow only some groups to use the client.

### Zitadel (untested)

- **Client:** in a project, New application › **Web**, authentication method **POST** (client secret in the body;
  PKCE also works), redirect URI `https://cloud.example.com/api/auth/oidc/callback` (development mode for an
  `http://` test address).
- **Issuer URL:** the instance's domain, e.g. `https://id.example.com`.
- **Claims:** `preferred_username` (login name; OxiCloud keeps the part before `@`), `email`, `email_verified`.
- **Groups:** Zitadel has no `groups` claim; project roles come as `urn:zitadel:iam:org:project:roles`. For admin
  groups, add a Zitadel Action (complement token flow) that sets a `groups` claim to a list of role or group
  names. Without it, leave Admin groups empty and make admins in Admin › Users.

### Kanidm (untested)

- **Client:** `kanidm system oauth2 create craftcloud "CraftCloud" https://cloud.example.com`, then
  `kanidm system oauth2 add-redirect-url craftcloud https://cloud.example.com/api/auth/oidc/callback` and
  `kanidm system oauth2 update-scope-map craftcloud <a group> openid profile email groups`. Kanidm requires PKCE
  (OxiCloud sends it) and signs with ES256 (accepted). `kanidm system oauth2 show-basic-secret craftcloud` prints
  the secret.
- **Issuer URL:** `https://idm.example.com/oauth2/openid/craftcloud`.
- **Claims:** `preferred_username` is the SPN (`name@idm.example.com`; OxiCloud keeps `name`), or the short name
  after `kanidm system oauth2 prefer-short-username craftcloud`. `email`, `email_verified`.
- **Groups:** with the `groups` scope, the `groups` claim lists group SPNs (`craftcloud-admins@idm.example.com`)
  and UUIDs: put the SPN form in Admin groups.

### Any other OpenID Connect provider

Anything that publishes `/.well-known/openid-configuration`, supports the authorization code flow with PKCE and
`client_secret_post`, and signs ID tokens with RS256/ES256 should work: register the redirect URI, request
`openid profile email` (plus whatever scope yields a `groups` list), and fill in the issuer URL, client ID and
secret.

### Google and Microsoft Entra ID (untested, with limits)

- **Google:** works as a plain provider: issuer `https://accounts.google.com`, an OAuth client of type "Web
  application" with the redirect URI. Google sends no `preferred_username` (the username is made from `name`) and
  no groups, so **Admin groups don't work**; make admins in Admin › Users. Any Google account can sign in unless
  the OAuth consent screen is limited to your Workspace ("Internal"), so think twice before leaving **Create
  accounts at first sign-in** on.
- **Microsoft Entra ID:** single-tenant only: issuer `https://login.microsoftonline.com/<tenant-id>/v2.0`
  (the multi-tenant `common`/`organizations` endpoints don't work: their tokens' issuer differs from the
  metadata's). Register a web app with the redirect URI and a client secret. Add the optional claim `email`;
  `preferred_username` is the UPN (OxiCloud keeps the part before `@`). Entra sends no `email_verified`, so
  existing accounts aren't joined by email (sign in creates a separate account, or is refused if account creation
  is off). A `groups` claim (Token configuration › Add groups claim) carries group **object IDs**: put the IDs in
  Admin groups.

## The desktop client

The desktop client signs in through the browser with a code (the device flow), so it works with single sign-on
too: sign in on the page it opens (with the provider's button), then approve the code. If the provider's
round trip lands you on the home page instead of the approval page, open the link the client shows again; you
are signed in by then.

## When it doesn't work

The sign-in page says what went wrong in plain words; the container log (`docker compose logs craftcloud`, lines
with `OIDC callback failed`) has OxiCloud's exact reason.

| The sign-in page says | Usually |
|---|---|
| "This server can't reach its identity provider right now." | The issuer URL is wrong, or the container can't reach it or doesn't trust its certificate. **TEST CONNECTION** shows the server's error. |
| "The identity provider's answer was refused. …" | A wrong client secret (log: `Token exchange failed with status 401`), no account while **Create accounts at first sign-in** is off (log: `Auto-provisioning is disabled`), or a sign-in page that was reloaded or used twice. |
| "The identity provider didn't sign you in …" | The person cancelled, or the provider doesn't let them use this client. |
| "The identity provider reported an error. …" | The provider refused the request, e.g. a redirect URI that doesn't match the registered one, or a scope it doesn't know. The provider's own log says which. |
| "Your identity provider hasn't confirmed your email address. …" | `email_verified` is false or missing where it matters (joining an existing account, or `OXICLOUD_REQUIRE_VERIFIED_EMAIL=true`). |
| "An account with your email address already exists here and can't be joined …" | Joining by email is off (`OXICLOUD_OIDC_AUTO_LINK_EMAIL_MATCH=false`), or that account is already linked to another identity. |
| "That single sign-on link expired (they last a minute)." | The one-time code was older than 60 seconds or used twice. Sign in again. |
| "Signed in, but this browser didn't keep the session" | The session cookies are marked Secure: open CraftCloud at its `https://` public address. |

Some of these pages would be raw error pages from OxiCloud (a provider's `?error=…` answer, an unreachable
provider); CraftCloud's Caddyfile sends them to the sign-in page instead (docs/UPSTREAM.md C14).

## What was tested

- **End to end, on 2026-11-14,** with the `craftcloud:main-check` image (OxiCloud 0.10.0) and Keycloak 26.0 in
  development mode, both in Docker on one host, configured through `PUT /api/admin/settings/oidc`, with headless
  Chromium: the button "Sign in with <name>", the redirect to Keycloak with PKCE (S256), state and nonce, the
  Keycloak sign-in, the callback and the code exchange, landing signed in on the home page. A user in the admin
  group was created as admin with the right email, a user outside it as a user. The secret is never sent back
  (`client_secret_set: true`). Password sign-in off through the admin settings applied at once
  (`POST /api/auth/login` refused) while single sign-on kept working.
- **Failures, end to end:** a wrong client secret and a person without an account with account creation off both
  came back as `callback_denied`; a provider `?error=access_denied`, a callback without parameters and an
  unreachable issuer came back as the sign-in page's keys through the Caddyfile (tested by loading the new
  Caddyfile into that container).
- **Automated:** the request builders and answers (`crates/core/src/api/oidc.rs`), the sign-in page's states and
  every error key (`crates/ui-egui/src/web/oidc/tests.rs`) and the admin panel's save, password switch and
  connection test (`crates/ui-egui/src/web/admin/tests.rs`), against answers recorded from OxiCloud 0.10.0. The
  frontend's new error texts were not part of the end-to-end run (the image predates them).
