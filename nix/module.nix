# NixOS module: services.craftcloud.
#
# Runs the CraftCloud image (OxiCloud + CraftCloud's frontend, admin service and Caddy) as an OCI
# container with NixOS's own PostgreSQL. The container reaches the database on the host; data lives
# in a volume (or a host path). Put a TLS reverse proxy (Caddy, nginx) in front for the internet.
#
#   services.craftcloud = {
#     enable = true;
#     publicUrl = "https://cloud.example.org";
#     environmentFile = "/run/secrets/craftcloud.env";   # POSTGRES_PASSWORD=…
#   };
{ config, lib, pkgs, ... }:

let
  cfg = config.services.craftcloud;
  pgReady = if config.systemd.services ? postgresql-setup then "postgresql-setup.service" else "postgresql.service";
in
{
  options.services.craftcloud = {
    enable = lib.mkEnableOption "CraftCloud, a self-hosted cloud for the Craft apps";

    image = lib.mkOption {
      type = lib.types.str;
      default = "ghcr.io/jonasbove/craftcloud:latest";
      description = ''
        The image to run. The `-apps` tag bundles the Craft web apps. A plain-HTTP registry needs
        `virtualisation.docker.daemon.settings.insecure-registries` (or podman's registries.conf).
      '';
    };

    imageFile = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      example = lib.literalExpression ''
        pkgs.dockerTools.pullImage {
          imageName = "ghcr.io/jonasbove/craftcloud";
          imageDigest = "sha256:…";
          hash = "sha256-…";
          finalImageTag = "0.1.0";
        }
      '';
      description = ''
        The image as a file in the Nix store, loaded instead of pulled from a registry (a machine
        without registry access; the flake's VM test). Its name and tag must match `image`.
      '';
    };

    publicUrl = lib.mkOption {
      type = lib.types.str;
      example = "https://cloud.example.org";
      description = "The address people type to reach CraftCloud (share links, desktop sign-in).";
    };

    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address CraftCloud listens on (keep 127.0.0.1 behind a reverse proxy; a tailnet or LAN address to reach it directly).";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8086;
      description = "Host port for CraftCloud.";
    };

    webApps = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Seed for the first start only; afterwards Admin › Server decides (needs an -apps image).";
    };

    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/craftcloud";
      description = "Where the files and CraftCloud's settings are kept (mounted at /app/storage).";
    };

    environmentFile = lib.mkOption {
      type = lib.types.path;
      description = ''
        A file with `POSTGRES_PASSWORD=…` (and any other `OXICLOUD_*` / `CRAFTCLOUD_*` settings),
        kept out of the Nix store.
      '';
    };

    cookieSecure = lib.mkOption {
      type = lib.types.bool;
      default = lib.hasPrefix "https://" cfg.publicUrl;
      defaultText = lib.literalExpression ''lib.hasPrefix "https://" publicUrl'';
      description = "Mark session cookies Secure (on by default for an https:// public URL).";
    };
  };

  config = lib.mkIf cfg.enable {
    services.postgresql = {
      enable = true;
      # OxiCloud needs these extensions (shipped with PostgreSQL).
      ensureDatabases = [ "craftcloud" ];
      ensureUsers = [{ name = "craftcloud"; ensureDBOwnership = true; }];
      settings.listen_addresses = lib.mkDefault "localhost";
      authentication = lib.mkAfter ''
        host craftcloud craftcloud 127.0.0.1/32 scram-sha-256
      '';
    };

    # Give the database user the password from the environment file, and the extensions.
    systemd.services.craftcloud-db-setup = {
      description = "CraftCloud: database password and extensions";
      # NixOS 25.11 and later create the database and the user (ensureDatabases, ensureUsers) in
      # postgresql-setup, after postgresql.service is up; before it has run, "database craftcloud
      # does not exist". Older NixOS does it in postgresql.service itself.
      after = [ pgReady ];
      requires = [ pgReady ];
      before = [ "${config.virtualisation.oci-containers.backend}-craftcloud.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        User = "postgres";
        EnvironmentFile = cfg.environmentFile;
      };
      path = [ config.services.postgresql.package ];
      script = ''
        # The password goes in as a psql variable, quoted by psql (:'pw'), never pasted into SQL.
        psql -v ON_ERROR_STOP=1 -v pw="$POSTGRES_PASSWORD" -d craftcloud <<'SQL'
        ALTER ROLE craftcloud WITH LOGIN PASSWORD :'pw';
        CREATE EXTENSION IF NOT EXISTS pg_trgm;
        CREATE EXTENSION IF NOT EXISTS ltree;
        SQL
      '';
    };

    systemd.tmpfiles.rules = [ "d ${cfg.dataDir} 0750 1001 1001 -" ];

    virtualisation.oci-containers.containers.craftcloud = {
      image = cfg.image;
      inherit (cfg) imageFile;
      # Host networking: the container reaches PostgreSQL on 127.0.0.1, and Caddy inside listens
      # on listenAddress:port directly.
      extraOptions = [ "--network=host" ];
      volumes = [ "${cfg.dataDir}:/app/storage" ];
      environmentFiles = [ cfg.environmentFile ];
      environment = {
        CRAFTCLOUD_PUBLIC_URL = cfg.publicUrl;
        CRAFTCLOUD_PORT = toString cfg.port;
        CRAFTCLOUD_BIND_ADDRESS = cfg.listenAddress;
        CRAFTCLOUD_WEB_APPS = if cfg.webApps then "on" else "off";
        OXICLOUD_COOKIE_SECURE = lib.boolToString cfg.cookieSecure;
      };
      # The database URL needs the password from the environment file, so the entrypoint is
      # wrapped to build it there.
      entrypoint = "/bin/sh";
      cmd = [ "-c" "export OXICLOUD_DB_CONNECTION_STRING=postgres://craftcloud:$POSTGRES_PASSWORD@127.0.0.1:5432/craftcloud; exec craftcloud-entrypoint" ];
    };

    # Host networking: OxiCloud and craftcloud-admin also listen on the host's 127.0.0.1:8087 and
    # :8088, so those ports must be free.
    networking.firewall.allowedTCPPorts = lib.mkIf (cfg.listenAddress != "127.0.0.1") [ cfg.port ];
  };
}
