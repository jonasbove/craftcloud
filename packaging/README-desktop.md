# CraftCloud desktop client

Keeps a folder on this computer in sync with your CraftCloud server, and is also the `craftcloud`
command line. These builds are **not code-signed** by Apple or Microsoft yet, so your system asks
before the first start:

- **macOS** (`craftcloud-<ver>-macos-universal.dmg`, Apple silicon and Intel): open the DMG, drag
  `CraftCloud.app` to Applications, then **right-click → Open → Open** the first time (Gatekeeper).
  Or run `xattr -dr com.apple.quarantine /Applications/CraftCloud.app`. The command line is
  `/Applications/CraftCloud.app/Contents/MacOS/craftcloud`.
- **Windows** (`craftcloud-<ver>-windows-{x64,arm64}.msi`): run the installer (no administrator
  rights needed). If SmartScreen says "Windows protected your PC", click **More info → Run anyway**.
  The `-portable.zip` holds the same `craftcloud.exe` to run without installing.
- **Linux:** the `.deb`, `.rpm`, `.pkg.tar.zst` (Arch), `.flatpak` or `.AppImage`. From the
  `.tar.gz`: unpack it and run `./install.sh` in the folder (to `~/.local`, no root;
  `./install.sh --remove` undoes it). On NixOS, add the flake to your system config instead (see the
  project README, "Nix and NixOS") so it updates with the rest of the system.

Then enter your server's address (with `http://` or `https://`), approve the device in the browser that
opens, and choose the folders to sync.

Every release lists the files' SHA-256 in `SHA256SUMS.txt`, and the release notes say how to check
them and their signatures: https://github.com/jonasbove/craftcloud/releases
