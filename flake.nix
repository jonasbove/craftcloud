{
  # Modelled on craft-launcher's flake.nix (MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team).
  #
  #   nix run github:jonasbove/craftcloud                                       # the desktop client
  #   nix develop                                                                # a shell for cargo
  #   nixosModules.default                                                       # services.craftcloud (server)
  description = "CraftCloud: desktop client, server module and tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    {
      # The server: the CraftCloud image plus NixOS's PostgreSQL (nix/module.nix).
      nixosModules.default = import ./nix/module.nix;
      nixosModules.craftcloud = self.nixosModules.default;
    } //
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        lib = pkgs.lib;

        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" "rust-src" ];
          targets = [ "wasm32-unknown-unknown" ];
        };
        # Packages build with the minimal toolchain (compiler and std only); the full one above is for
        # the dev shell. The binaries must not keep any toolchain alive: std's source paths in panic
        # messages would otherwise make the ~2 GB toolchain a runtime dependency (see postFixup).
        rustMinimal = pkgs.rust-bin.stable.latest.minimal;
        rustPlatform = pkgs.makeRustPlatform { cargo = rustMinimal; rustc = rustMinimal; };
        dropToolchain = bin: ''
          remove-references-to -t ${rustMinimal} ${bin}
        '';

        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        # The window (winit, wgpu/GL) and the tray (pure-Rust D-Bus) need no GTK or libdbus.
        runtimeLibs = with pkgs; [
          wayland
          libxkbcommon
          libGL
          vulkan-loader
          libx11
          libxcursor
          libxrandr
          libxi
          libxcb
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [ rust pkgs.pkg-config pkgs.trunk ];
          buildInputs = runtimeLibs;
          LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
        };

        packages.default = rustPlatform.buildRustPackage {
          pname = "craftcloud";
          inherit version;
          # Only what the Rust workspace builds from (not the design files or upstream/).
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./.cargo ./crates ./apps ./xtask ./packaging/linux ];
          };
          cargoLock = {
            lockFile = ./Cargo.lock;
            # Git dependencies (the storytold egui/winit forks, craft-libs) at their locked revisions.
            allowBuiltinFetchGit = true;
          };
          cargoBuildFlags = [ "-p" "craftcloud" ];
          nativeBuildInputs = [ pkgs.pkg-config pkgs.autoPatchelfHook pkgs.removeReferencesTo ];
          # libgcc_s for the unwinder, as every Rust binary needs.
          buildInputs = runtimeLibs ++ [ pkgs.stdenv.cc.cc.lib ];
          runtimeDependencies = runtimeLibs;
          # `Open folder` uses the desktop's own xdg-open (bundling xdg-utils would add Perl, ~60 MB).
          # The app menu entry, its AppStream file and icons, all named for the window's app id
          # (io.github.jonasbove.craftcloud).
          postInstall = dropToolchain "$out/bin/craftcloud" + ''
            id=io.github.jonasbove.craftcloud
            install -Dm644 packaging/linux/$id.desktop $out/share/applications/$id.desktop
            mkdir -p $out/share/metainfo $out/share/icons
            sed -e "s/@VERSION@/${version}/g" -e "s/@DATE@/1970-01-01/g" \
              packaging/linux/$id.metainfo.xml.in > $out/share/metainfo/$id.metainfo.xml
            cp -R packaging/linux/icons/hicolor $out/share/icons/
          '';
          disallowedReferences = [ rustMinimal ];
          doCheck = false;
          meta.mainProgram = "craftcloud";
        };

        # craftcloud-admin, the image's small server-side service (for running outside the image).
        packages.craftcloud-admin = rustPlatform.buildRustPackage {
          pname = "craftcloud-admin";
          inherit version;
          src = self.packages.${system}.default.src;
          cargoLock = { lockFile = ./Cargo.lock; allowBuiltinFetchGit = true; };
          cargoBuildFlags = [ "-p" "craftcloud-admin" ];
          nativeBuildInputs = [ pkgs.removeReferencesTo ];
          postInstall = dropToolchain "$out/bin/craftcloud-admin";
          disallowedReferences = [ rustMinimal ];
          doCheck = false;
          meta.mainProgram = "craftcloud-admin";
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/craftcloud";
        };

        # A NixOS system using the server module must evaluate (catches option and type mistakes).
        checks.module = (nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            self.nixosModules.default
            {
              boot.loader.grub.enable = false;
              fileSystems."/" = { device = "none"; fsType = "tmpfs"; };
              system.stateVersion = "25.11";
              virtualisation.docker.enable = true;
              virtualisation.oci-containers.backend = "docker";
              services.craftcloud = {
                enable = true;
                publicUrl = "https://cloud.example.org";
                environmentFile = "/run/secrets/craftcloud.env";
              };
            }
          ];
        }).config.system.build.toplevel;
      });
}
