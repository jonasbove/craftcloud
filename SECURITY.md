# Security

CraftCloud keeps people's files and sign-ins, so security reports are welcome and handled before anything else.

## Reporting a vulnerability

Report it **privately**, through GitHub: the repository's **Security** tab › **Report a vulnerability**
(https://github.com/jonasbove/craftcloud/security/advisories/new). Please don't open a public issue or pull request
for it.

Say what you found, how to reproduce it, the version (`craftcloud --version`, or the image tag) and what an attacker
could do with it. You'll get an answer within a week. Once it's fixed, the advisory is published with the release
that fixes it, crediting you unless you'd rather not be named.

## What is covered

- CraftCloud's own code: the desktop client and command line, the web frontend, `craftcloud-admin`, the image's
  Caddy configuration and entrypoint, and the packaging.
- The way CraftCloud runs OxiCloud and Caddy. A vulnerability in OxiCloud itself belongs with its project
  (https://github.com/AtalayaLabs/OxiCloud); if you're not sure where it belongs, report it here.

## Supported versions

Only the latest release gets security fixes. CraftCloud is an early alpha, so update when a release says it fixes
a security problem.

## Checking what you download

Release files come with `SHA256SUMS.txt`, GPG signatures (`.asc`, made with the key `97062AFE35FFD84487658C6DD11DD5C65CF191CE` in `release-signing-key.asc`) and signed
build-provenance attestations; the images on `ghcr.io/jonasbove/craftcloud` carry attestations too. The README's
**Downloads** section says how to check them.
