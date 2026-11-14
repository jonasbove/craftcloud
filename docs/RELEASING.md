# Releasing CraftCloud

How a version goes out. Everything happens on https://github.com/jonasbove/craftcloud: the code, its pull requests,
the tags, the releases and the images.

## What a release is

A `v<version>` tag on `main`, whose version matches `Cargo.toml`. Pushed to GitHub, the tag starts two workflows:

- **Release** (`.github/workflows/release.yml`): the desktop packages for Linux, Windows and macOS, and a **draft**
  GitHub release titled `CraftCloud v<version>`, with the files, `SHA256SUMS.txt`, GPG signatures (`.asc`),
  `release-signing-key.asc` and build-provenance attestations. Its text has the Craft apps' layout: `## What's
  Changed` (GitHub's generated notes: each pull request merged since the previous release, with its author) and
  `**Full Changelog**`, plus a link to
  [RELEASE-NOTES.md](RELEASE-NOTES.md) when it has a section for the version.
  FreeBSD and riscv64 are added to the draft later by the `late` job, when (and if) they build.
  The draft is made only when the **package tests** pass (`package-test`, `packaging/linux/test-package.sh`): every
  Linux package is installed in a clean container with its distro's package manager (`.deb` on Debian 12, Ubuntu
  22.04 and 24.04; `.rpm` on Fedora 41 and 42; both on x86_64 and aarch64; the Arch package on Arch Linux; the
  AppImage and the tarball on Ubuntu 22.04), checked (`--version`, `--help`, the desktop entry, icons, AppStream
  file and `craftcloud://` handler), signed in through the device flow and synced both ways against a server image
  built from the same commit, upgraded from the previous published release, and removed again.
- **Server image** (`.github/workflows/image.yml`): `ghcr.io/jonasbove/craftcloud:<version>`, `:latest`,
  `:<version>-apps` and `:latest-apps`, for amd64 and arm64, with attestations, and the same images copied to
  Docker Hub (`docker.io/jonasbove/craftcloud`). These are public as soon as the
  workflow finishes.

Nothing is released by itself: a person publishes the draft.

## Checklist

Steps 1 and 2 are one pull request ("🔖 Release <version>"), merged like any other: by a maintainer's explicit
confirmation.

1. **Release notes:** turn `## Unreleased` in [RELEASE-NOTES.md](RELEASE-NOTES.md) into `## <version>`, with what
   changed and, first, anything that needs care when updating ("upgrade all devices together"). Start a new empty
   `## Unreleased` above it.
2. **Version:** set `version` in the root `Cargo.toml` and run `cargo update -w`. Update the stage line in
   [ROADMAP.md](ROADMAP.md) and the README's Status section if they name the version.
3. **Gate:** `cargo xtask ci` must say `all 4 steps passed`. Build the image and run `scripts/smoke-test` on it.
   To try the whole release without publishing anything, start the Release workflow by hand (on `main` or a
   branch): it builds every file and runs the package tests, and makes no draft (that needs a tag).
4. **Before a bigger release, on a desktop:** install the new packages on a Fedora VM (`.rpm`) and a Debian VM
   (`.deb`), each with a desktop session, and check what containers can't: the tray icon and its menu, the sign-in
   kept in the keychain across a restart, the browser sign-in (the code page opens and approving it signs in), and
   start at login. A `craftcloud://` link from the browser opening the client is worth a click too.
5. **Tag** `main` after the release pull request is merged: `git pull`, then
   `git tag -a v<version> -m "CraftCloud <version>"` on the merge commit and `git push origin v<version>`. Tags
   can't be moved or deleted afterwards, so check the commit first.
6. **Watch** the Release and Server image runs. A failed package test means no draft: its log shows the distro,
   the step and, for a sync failure, the server's log; fix it and tag a new version. When the draft has every file
   (and FreeBSD/riscv64, or their jobs have given up), read it over and **publish** it. Published releases are immutable: their files can't be
   changed afterwards.
7. **After publishing:**
   - Update the AUR recipes in `packaging/arch/` (`pkgver`, `updpkgsums`, `makepkg --printsrcinfo > .SRCINFO`) and
     push them to the AUR.
   - Redeploy your own servers from the new image, after reading the release notes.

## The signing key

- Releases are signed with the key `97062AFE35FFD84487658C6DD11DD5C65CF191CE` ("CraftCloud releases"); its public
  part is [release-signing-key.asc](../release-signing-key.asc).
- The main key is certify-only and kept offline. GitHub holds only a signing subkey, without a passphrase, as the
  `RELEASE_SIGNING_KEY` secret of the `release` environment, which only `v*` tags may use.
- **The subkey expires on 2028-11-13.** Before then, make a new signing subkey with the main key, put it in the
  secret, and publish the updated public key (in the repository and on keys.openpgp.org). If the subkey leaks,
  revoke it with the main key, do the same, and say so in the next release's notes.

## GitHub settings the release relies on

- Environment `release`: deployments only from tags matching `v*`; holds `RELEASE_SIGNING_KEY`.
- Repository variable `DOCKERHUB_USERNAME` and secret `DOCKERHUB_TOKEN` (a Docker Hub access token with read and
  write access) for the Docker Hub copy.
- Rulesets: `v*` tags can't be moved or deleted; `main` changes only through pull requests (no approvals required,
  no bypass) and can't be force-pushed or deleted.
- Immutable releases, secret scanning with push protection, Dependabot alerts, private vulnerability reporting
  ([SECURITY.md](../SECURITY.md)).
- Workflows from outside contributors' pull requests wait for approval; `GITHUB_TOKEN` is read-only by default.
- Every pull request, from anyone, is merged on GitHub by a maintainer's explicit confirmation (AGENTS.md).
