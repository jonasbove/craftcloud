# Releasing CraftCloud

How a version goes out. The primary repository keeps the full history; https://github.com/jonasbove/craftcloud is a
public copy whose history starts at the first release (`scripts/publish-github`), and that is where releases and
images are published.

## What a release is

A `v<version>` tag on `main`, whose version matches `Cargo.toml`. Pushed to GitHub, the tag starts two workflows:

- **Release** (`.github/workflows/release.yml`): the desktop packages for Linux, Windows and macOS, and a **draft**
  GitHub release titled `CraftCloud v<version>`, with the files, `SHA256SUMS.txt`, GPG signatures (`.asc`),
  `release-signing-key.asc` and build-provenance attestations. Its text has the Craft apps' layout: `## What's
  Changed` (the commits since the previous release, one line each) and `**Full Changelog**`, plus a link to
  [RELEASE-NOTES.md](RELEASE-NOTES.md) when it has a section for the version.
  FreeBSD and riscv64 are added to the draft later by the `late` job, when (and if) they build.
- **Server image** (`.github/workflows/image.yml`): `ghcr.io/jonasbove/craftcloud:<version>`, `:latest`,
  `:<version>-apps` and `:latest-apps`, for amd64 and arm64, with attestations, and the same images copied to
  Docker Hub (`docker.io/jonasbove/craftcloud`). These are public as soon as the
  workflow finishes.

Nothing is released by itself: a person publishes the draft.

## Checklist

1. **Release notes:** turn `## Unreleased` in [RELEASE-NOTES.md](RELEASE-NOTES.md) into `## <version>`, with what
   changed and, first, anything that needs care when updating ("upgrade all devices together"). Start a new empty
   `## Unreleased` above it.
2. **Version:** set `version` in the root `Cargo.toml` and run `cargo update -w`. Update the stage line in
   [ROADMAP.md](ROADMAP.md) and the README's Status section if they name the version.
3. **Gate:** `cargo xtask ci` must say `all 4 steps passed`. Build the image and run `scripts/smoke-test` on it.
4. **Tag** the commit on `main` (`git tag -a v<version> -m "CraftCloud <version>"`) and push the tag to the primary
   repository.
5. **Publish** to GitHub as the project account (no private address in the public history):

   ```sh
   PUBLIC_NAME=jonasbove PUBLIC_EMAIL=31095576+jonasbove@users.noreply.github.com \
     scripts/publish-github https://github.com/jonasbove/craftcloud.git v0.1.0
   ```

   The second argument is always `v0.1.0`, where the public history starts; the script replays the commits since
   the last run and pushes the tags that point at them.
6. **Watch** the Release and Server image runs. When the draft has every file (and FreeBSD/riscv64, or their jobs
   have given up), read it over and **publish** it. Published releases are immutable: their files can't be
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
- A ruleset protecting `v*` tags from being moved or deleted, and `main` from force pushes and deletion.
- Immutable releases, secret scanning with push protection, Dependabot alerts, private vulnerability reporting
  ([SECURITY.md](../SECURITY.md)).
- Workflows from outside contributors' pull requests wait for approval; `GITHUB_TOKEN` is read-only by default.
- Changes reach GitHub only through `scripts/publish-github`, so pull requests opened there are merged in the
  primary repository and published from it, not merged on GitHub.
