# Arch Linux packages

- **`package.sh`**: the release's own `craftcloud-<ver>-linux-<arch>.pkg.tar.zst`, built with
  `makepkg` from the Linux tarball (`.github/workflows/release.yml` runs it in an
  `archlinux:base-devel` container). Install it with `sudo pacman -U craftcloud-<ver>-linux-<arch>.pkg.tar.zst`.
- **`craftcloud-bin/PKGBUILD`**: an AUR recipe for the prebuilt release tarball. It checks the
  tarball's SHA-256 and its GPG signature (`.asc`), against the release signing key
  (`_fingerprint`, the key in `release-signing-key.asc`). Import it once before building:
  `gpg --import release-signing-key.asc`.
- **`craftcloud/PKGBUILD`**: an AUR recipe that builds the client from the tagged source with cargo.

Both AUR recipes conflict with each other and provide `craftcloud`.

## Updating an AUR recipe for a release

In a clean checkout of the AUR repository (`ssh://aur@aur.archlinux.org/<name>.git`), with this
folder's `PKGBUILD` copied in:

```sh
sed -i 's/^pkgver=.*/pkgver=0.2.0/; s/^pkgrel=.*/pkgrel=1/' PKGBUILD
updpkgsums                      # downloads the sources and fills in sha256sums (pacman-contrib)
makepkg -si                     # builds and installs it, checking the hashes and the signature
namcap PKGBUILD *.pkg.tar.zst   # optional lint
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO && git commit -m "Update to 0.2.0" && git push
```

The `.SRCINFO` is generated, never edited by hand; regenerate it after every change to the
`PKGBUILD`. Publishing to the AUR is a maintainer's decision.
