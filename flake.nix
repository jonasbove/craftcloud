{
  # Modelled on craft-launcher's flake.nix (MIT OR Apache-2.0, Copyright (c) 2026 ArtCraft Team).
  #
  #   nix run github:jonasbove/craftcloud                                       # the desktop client
  #   nix develop                                                                # a shell for cargo
  #   nixosModules.default                                                       # services.craftcloud (server)
  #   homeManagerModules.default                                                 # programs.craftcloud (desktop client)
  #   nix build .#checks.x86_64-linux.vm-sync -L                                 # the server module and a client, in VMs
  description = "CraftCloud: desktop client, server module and tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Only for the Home Manager module's check; a user's own Home Manager evaluates the module.
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, home-manager }:
    {
      # The server: the CraftCloud image plus NixOS's PostgreSQL (nix/module.nix).
      nixosModules.default = import ./nix/module.nix;
      nixosModules.craftcloud = self.nixosModules.default;

      # The desktop client for Home Manager (nix/home-manager.nix).
      homeManagerModules.default = import ./nix/home-manager.nix self;
      homeManagerModules.craftcloud = self.homeManagerModules.default;
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

        # The server module in a VM, and a second VM signing in and syncing a file each way
        # (nix/vm-test.nix).
        checks.vm-sync = import ./nix/vm-test.nix {
          inherit pkgs;
          serverModule = self.nixosModules.default;
          client = self.packages.${system}.default;
        };

        # The Home Manager module must evaluate and write what it promises: the client installed,
        # the client's own autostart entry with --hidden, and the craftcloud:// default. Nothing
        # is built (the package is only named).
        checks.home-manager =
          let
            home = (home-manager.lib.homeManagerConfiguration {
              inherit pkgs;
              modules = [
                self.homeManagerModules.default
                {
                  home.username = "john";
                  home.homeDirectory = "/home/john";
                  home.stateVersion = "25.11";
                  programs.craftcloud = { enable = true; startAtLogin = true; };
                }
              ];
            }).config;
            entry = home.xdg.configFile."autostart/io.github.jonasbove.craftcloud.desktop";
          in
          assert builtins.elem self.packages.${system}.default home.home.packages;
          assert lib.hasInfix "Exec=\"craftcloud\" --hidden\n" entry.text;
          assert home.xdg.mimeApps.enable;
          assert home.xdg.mimeApps.defaultApplications."x-scheme-handler/craftcloud" == [ "io.github.jonasbove.craftcloud.desktop" ];
          # Forces the whole configuration to evaluate without building it.
          pkgs.writeText "craftcloud-home-manager-check"
            (builtins.unsafeDiscardStringContext home.home.activationPackage.drvPath);
      });
}
