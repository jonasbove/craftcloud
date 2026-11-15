# Home Manager module: programs.craftcloud, the desktop client.
#
# Installs the client (its menu entry, icons and the craftcloud:// handler come with the package),
# makes it the default for craftcloud:// links, and can start it in the tray at login.
#
#   programs.craftcloud = {
#     enable = true;
#     startAtLogin = true;   # craftcloud --hidden, from ~/.config/autostart
#   };
#
# The flake passes itself in, so `package` defaults to the flake's own build.
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.craftcloud;
  id = "io.github.jonasbove.craftcloud";
in
{
  options.programs.craftcloud = {
    enable = lib.mkEnableOption "the CraftCloud desktop client";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "craftcloud.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The CraftCloud package to install.";
    };

    startAtLogin = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Start CraftCloud in the tray (`--hidden`) when you log in. This writes the same XDG autostart
        entry as the client's own Settings › Start at login (`~/.config/autostart/${id}.desktop`),
        so the setting shows it as on and the client never starts twice. Turning it off in the app
        lasts until the next switch. Off leaves the entry to the app's setting.
      '';
    };

    defaultForLinks = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Make CraftCloud the default application for `craftcloud://` links
        (`x-scheme-handler/craftcloud` in `~/.config/mimeapps.list`; turns on `xdg.mimeApps`).
      '';
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    { home.packages = [ cfg.package ]; }

    (lib.mkIf cfg.startAtLogin {
      # The client's own entry, word for word (crates/native/src/login.rs): `craftcloud` from PATH,
      # as a store path changes on every update. `force`: turning the setting off and on again in
      # the app leaves a file of its own there, which the next switch replaces.
      xdg.configFile."autostart/${id}.desktop" = {
        force = true;
        text = ''
          [Desktop Entry]
          Type=Application
          Name=CraftCloud
          Comment=Keeps your CraftCloud folder in sync
          Exec="craftcloud" --hidden
          Terminal=false
          X-GNOME-Autostart-enabled=true
        '';
      };
    })

    (lib.mkIf cfg.defaultForLinks {
      xdg.mimeApps = {
        enable = lib.mkDefault true;
        defaultApplications."x-scheme-handler/craftcloud" = [ "${id}.desktop" ];
      };
    })
  ]);
}
