# NixOS VM test: the server module boots the released image with NixOS's PostgreSQL, and a second
# machine signs in with the command line (the OAuth device flow, approved over the API as the
# /device page would) and syncs a file each way with `craftcloud sync --once`.
#
#   nix build .#checks.x86_64-linux.vm-sync -L      (needs KVM; about 3 GB of RAM for the two VMs)
#
# The image comes from the registry by digest (a fixed-output download), so the test runs offline
# inside the VMs; the client and the module are this tree's.
{ pkgs, serverModule, client }:

let
  # The released server image, ghcr.io/jonasbove/craftcloud:0.1.0 (the multi-arch digest; the hash
  # is of the image for this system's architecture).
  image = pkgs.dockerTools.pullImage {
    imageName = "ghcr.io/jonasbove/craftcloud";
    imageDigest = "sha256:a15b2f27a03896740c757c49de011c5927fefdcbf3978813fa4bbe18578aa1ab";
    hash = {
      x86_64-linux = "sha256-Tr2rma367kiIwugFvl8hdQUvpIDbQCx8rWQ2ugh4cIU=";
      aarch64-linux = "sha256-/e/XFsVXNHOJlwQLT+6zAt5A9eLVqgQDrt2uMcYXslQ=";
    }.${pkgs.stdenv.hostPlatform.system};
    finalImageName = "ghcr.io/jonasbove/craftcloud";
    finalImageTag = "0.1.0";
    arch = if pkgs.stdenv.hostPlatform.isAarch64 then "arm64" else "amd64";
  };
in
pkgs.testers.runNixOSTest {
  name = "craftcloud-vm-sync";

  nodes.server = { ... }: {
    imports = [ serverModule ];
    virtualisation.memorySize = 2048;
    virtualisation.diskSize = 4096;
    # A throwaway database password, only for this test.
    environment.etc."craftcloud.env".text = "POSTGRES_PASSWORD=vm-test-only\n";
    services.craftcloud = {
      enable = true;
      image = "ghcr.io/jonasbove/craftcloud:0.1.0";
      imageFile = image;
      publicUrl = "http://server:8086";
      listenAddress = "0.0.0.0";
      environmentFile = "/etc/craftcloud.env";
    };
    environment.systemPackages = [ pkgs.curl ];
  };

  nodes.laptop = { ... }: {
    environment.systemPackages = [ client pkgs.curl ];
  };

  testScript = ''
    import json
    import shlex

    url = "http://server:8086"
    user, password = "john", "vm-test-password"
    # The client's settings and tokens in a folder of their own, no keychain (no desktop session).
    cc = "env CRAFTCLOUD_CONFIG_DIR=/root/cc CRAFTCLOUD_KEYCHAIN=off craftcloud"
    folder = "/root/CraftCloud"

    def api(path, body, token=None):
        auth = f"-H 'Authorization: Bearer {token}' " if token else ""
        return server.succeed(
            f"curl -fsS {auth}-H 'Content-Type: application/json' "
            f"-d {shlex.quote(json.dumps(body))} http://127.0.0.1:8086{path}"
        )

    def dav(args, token):
        return server.succeed(f"curl -fsS -H 'Authorization: Bearer {token}' {args}")

    start_all()

    with subtest("the server starts"):
        server.wait_for_unit("podman-craftcloud.service")
        server.wait_until_succeeds("curl -fsS http://127.0.0.1:8086/ready", timeout=300)
        # The database setup is a oneshot that has finished by now: it must have succeeded.
        server.succeed("systemctl show -p Result craftcloud-db-setup.service | grep -qx Result=success")
        laptop.wait_for_unit("multi-user.target")
        laptop.wait_until_succeeds(f"curl -fsS {url}/api/auth/status", timeout=60)

    with subtest("a test owner is created"):
        status = json.loads(server.succeed("curl -fsS http://127.0.0.1:8086/api/auth/status"))
        assert status["initialized"] is False, status
        api("/api/setup", {"username": user, "email": "john@example.org", "password": password})
        token = json.loads(api("/api/auth/login", {"username": user, "password": password}))["access_token"]

    with subtest("the laptop signs in through the device flow"):
        # In the background, all of its output away from the driver's shell (which waits for it).
        laptop.succeed(
            f"setsid -f sh -c '{cc} login {url} --no-browser --json >/tmp/login.out 2>/tmp/login.err;"
            " echo $? >/tmp/login.status' </dev/null >/dev/null 2>&1"
        )
        laptop.wait_until_succeeds("test -s /tmp/login.out", timeout=60)
        code = json.loads(laptop.succeed("head -1 /tmp/login.out"))["user_code"]
        api("/api/auth/device/verify", {"user_code": code, "action": "approve"}, token)
        laptop.wait_until_succeeds("test -s /tmp/login.status", timeout=60)
        laptop.succeed("grep -qx 0 /tmp/login.status || { cat /tmp/login.out /tmp/login.err; false; }")
        signed_in = json.loads(laptop.succeed("sed -n 2p /tmp/login.out"))
        assert signed_in["signed_in"] is True and signed_in["username"] == user, signed_in
        # The sync folder, as the desktop client's setup would choose it.
        laptop.succeed(
            f"mkdir -p {folder} && sed -i '/^local_folder *=/d' /root/cc/config.toml"
            f" && echo 'local_folder = \"{folder}\"' >>/root/cc/config.toml"
        )

    with subtest("a file put in the cloud comes down"):
        dav("-X MKCOL http://127.0.0.1:8086/webdav/vmtest/", token)
        server.succeed("echo 'written in the cloud' >/tmp/from-cloud.txt")
        dav("-T /tmp/from-cloud.txt http://127.0.0.1:8086/webdav/vmtest/from-cloud.txt", token)
        out = json.loads(laptop.succeed(f"{cc} sync --once --json"))
        assert out["state"] == "up_to_date", out
        laptop.succeed(f"grep -qx 'written in the cloud' {folder}/vmtest/from-cloud.txt")

    with subtest("a file written in the sync folder goes up"):
        laptop.succeed(f"echo 'written on the laptop' >{folder}/vmtest/from-laptop.txt")
        # The sync leaves a file written in the last 2 seconds for the next pass (SETTLE_MS).
        laptop.succeed("sleep 3")
        out = json.loads(laptop.succeed(f"{cc} sync --once --json"))
        assert out["state"] == "up_to_date", out
        got = dav("http://127.0.0.1:8086/webdav/vmtest/from-laptop.txt", token)
        assert got == "written on the laptop\n", repr(got)
  '';
}
