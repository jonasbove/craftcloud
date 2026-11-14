# CraftCloud

**Your own cloud for the Craft apps: self-hosted file sync, built in Rust.** Files live on your server and in a
CraftCloud folder on each computer, synced both ways, so the Craft apps open and save them as ordinary local files.

This image is the server: the unmodified [OxiCloud](https://github.com/AtalayaLabs/OxiCloud) server behind Caddy,
with CraftCloud's own web frontend (first-run setup, files, sharing, versions, admin, optional single sign-on).
The desktop client for Linux, Windows and macOS, which is also the `craftcloud` command line, is on the
[releases page](https://github.com/jonasbove/craftcloud/releases).

**Early alpha:** back up what matters ([backups](https://github.com/jonasbove/craftcloud/blob/main/docs/BACKUP.md)).

## Tags

| Tag | What it is |
|---|---|
| `<version>`, `latest` | The server |
| `<version>-apps`, `latest-apps` | The server with the Craft apps for the browser (turn them on with `CRAFTCLOUD_WEB_APPS=on`) |

Every tag is for `linux/amd64` and `linux/arm64`, and the same images are on `ghcr.io/jonasbove/craftcloud`. Each
image carries a signed build-provenance attestation:
`gh attestation verify oci://docker.io/jonasbove/craftcloud:<version> --repo jonasbove/craftcloud`.

## Run it

CraftCloud runs as two containers, itself and PostgreSQL:

```sh
git clone https://github.com/jonasbove/craftcloud && cd craftcloud
cp .env.example .env   # set POSTGRES_PASSWORD and CRAFTCLOUD_PUBLIC_URL
echo "CRAFTCLOUD_IMAGE=jonasbove/craftcloud:latest" >> .env
docker compose up -d
```

Then open the public URL and follow the setup. Put a TLS reverse proxy in front for anything beyond a private
network. The [README](https://github.com/jonasbove/craftcloud#run-the-server) has every setting, updating and the
NixOS module.

## Links

- Source and issues: https://github.com/jonasbove/craftcloud
- Release notes: https://github.com/jonasbove/craftcloud/blob/main/docs/RELEASE-NOTES.md
- Security: https://github.com/jonasbove/craftcloud/security/policy

CraftCloud is MIT OR Apache-2.0. It is not made by or affiliated with the ArtCraft team or the OxiCloud project.
