//! The command line end to end against a real CraftCloud server. Skipped unless these are set
//! (never in CI):
//!
//! ```text
//! CRAFTCLOUD_TEST_SERVER=http://127.0.0.1:8095 CRAFTCLOUD_TEST_USER=john \
//! CRAFTCLOUD_TEST_PASSWORD=… cargo test -p craftcloud --test cli_live
//! ```
//!
//! It signs in through the device flow (approving the code with a password sign-in, as the browser
//! would), keeps its settings in a temporary folder and its tokens out of the keychain
//! (`CRAFTCLOUD_KEYCHAIN=off`), and works in a folder of its own, which it deletes at the end. Use
//! a throwaway server.
//!
//! With `CRAFTCLOUD_TEST_KIND=nextcloud|owncloud|opencloud|webdav` and `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1`
//! ([`kind`]), [`the_command_line_on_another_kind_of_server`] runs instead: `login --kind webdav`
//! with an app password, `server check` with the mode expected of the server, the file commands
//! where two-way sync is allowed, and the answers of the commands that are OxiCloud's only.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth;
use craftcloud_core::storage::Mode;
use craftcloud_native::http::execute;
use serde_json::Value;

#[path = "../../../crates/native/tests/live/kind.rs"]
mod kind;

struct Env {
    server: String,
    config: tempfile::TempDir,
    work: tempfile::TempDir,
    /// More variables for every command (the opt-in, the app password).
    extra: Vec<(&'static str, String)>,
}

impl Env {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_craftcloud"));
        c.args(args)
            .env("CRAFTCLOUD_CONFIG_DIR", self.config.path())
            .env("CRAFTCLOUD_KEYCHAIN", "off")
            .env_remove("CRAFTCLOUD_SERVER")
            .env_remove("CRAFTCLOUD_EXPERIMENTAL_WEBDAV")
            .env_remove("CRAFTCLOUD_APP_PASSWORD")
            .current_dir(self.work.path())
            .stdin(Stdio::null());
        for (name, value) in &self.extra {
            c.env(name, value);
        }
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    /// Runs a command that must work; its stdout as JSON (`--json` is added).
    fn ok(&self, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.push("--json");
        let out = self.run(&all);
        assert!(out.status.success(), "{args:?} failed ({:?}): {}", out.status.code(), String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(text.lines().last().unwrap_or("null")).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {text}"))
    }

    /// Runs a command that must fail with `code`; its JSON error.
    fn fails(&self, code: i32, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.push("--json");
        let out = self.run(&all);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {err}");
        serde_json::from_str(err.lines().last().unwrap_or("null")).unwrap_or(Value::Null)
    }
}

fn paths(list: &Value) -> Vec<String> {
    list.as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap().to_owned()).collect()
}

/// Signs the command line in: starts `login`, approves its code as the browser page would.
fn login(env: &Env, user: &str, password: &str) {
    let mut child = env.cmd(&["login", &env.server, "--no-browser", "--json"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let start: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let code = start["user_code"].as_str().unwrap();
    assert!(start["verification_uri"].as_str().unwrap().contains("/device?code="));

    let server = ServerUrl::parse(&env.server).unwrap();
    if !auth::parse_status(&execute(&auth::status(&server)).unwrap()).unwrap().initialized {
        auth::parse_setup(&execute(&auth::setup(&server, user, &format!("{user}@example.org"), password)).unwrap()).unwrap();
    }
    let tokens = auth::parse_login(&execute(&auth::login(&server, user, password)).unwrap()).unwrap();
    let approve = execute(&tokens.authorize(auth::device_verify(&server, code, true))).unwrap();
    assert!(approve.is_success(), "approving: {}", approve.status);

    let done: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(done["signed_in"], true);
    assert_eq!(done["username"], user);
    assert!(child.wait().unwrap().success());
}

/// Make, list and remove a link to the file `b` in the folder `docs`.
fn share_links(env: &Env, docs: &str, b: &str) {
    let link = env.ok(&["share", "create", b, "--expires", "7d"]);
    assert!(link["url"].as_str().unwrap().contains("/s/"));
    assert!(link["expires_at"].as_str().is_some());
    assert_eq!(env.ok(&["share", "ls", b]).as_array().unwrap().len(), 1);
    // T-040: every link says where its item is now, not the name it was shared with.
    let moved = format!("{docs}/moved.txt");
    env.ok(&["mv", b, &moved]);
    let all = env.ok(&["share", "ls"]);
    let mine = all.as_array().unwrap().iter().find(|l| l["id"] == link["id"]).unwrap().clone();
    assert_eq!((mine["path"].as_str(), mine["name"].as_str()), (Some(moved.as_str()), Some("moved.txt")), "{mine}");
    env.ok(&["mv", &moved, b]);
    env.ok(&["share", "rm", link["id"].as_str().unwrap()]);
    assert!(env.ok(&["share", "ls", b]).as_array().unwrap().is_empty());
}

/// T-047: restoring onto a place that is taken is refused (5) with a way out, not a server error;
/// `--rename` restores it next to it, `--as` where asked.
fn restore_onto_a_taken_place(env: &Env, docs: &str, a: &str, c: &str) {
    env.ok(&["rm", c]);
    env.ok(&["cp", a, c]);
    let refused = env.fails(5, &["trash", "restore", c]);
    assert!(refused["error"]["message"].as_str().unwrap().contains("--rename"), "{refused}");
    let back = env.ok(&["trash", "restore", c, "--rename"]);
    let renamed = format!("{docs}/c (restored).txt");
    assert_eq!(back["restored"].as_str(), Some(renamed.as_str()), "{back}");
    let listed = paths(&env.ok(&["ls", docs]));
    assert!(listed.contains(&renamed) && listed.contains(&c.to_owned()), "{listed:?}");
    env.ok(&["rm", &renamed]);
    let elsewhere = format!("{docs}/elsewhere.txt");
    assert_eq!(env.ok(&["trash", "restore", &renamed, "--as", &elsewhere])["restored"].as_str(), Some(elsewhere.as_str()));
    env.ok(&["rm", &elsewhere]);
}

/// T-050: a new file where another one was doesn't inherit that one's versions.
fn a_new_file_starts_clean(env: &Env, docs: &str) {
    let x = format!("{docs}/x.txt");
    std::fs::write(env.work.path().join("x.txt"), "first").unwrap();
    env.ok(&["put", "x.txt", &x]);
    env.ok(&["put", "x.txt", &x, "--force"]);
    assert_eq!(env.ok(&["versions", &x]).as_array().unwrap().len(), 1);
    env.ok(&["rm", &x]);
    env.ok(&["put", "x.txt", &x]);
    assert!(env.ok(&["versions", &x]).as_array().unwrap().is_empty(), "no versions of the file that was there before");
    env.ok(&["rm", &x]);
}

/// What a cloud backend runs (docs/CRAFT-INTEGRATION.md § 3.5): the file's version tag from
/// `ls`/`files --json`, its bytes from `get -`, and a save that writes only over that version.
fn save_over_the_version_read(env: &Env, docs: &str) {
    let n = format!("{docs}/n.md");
    std::fs::write(env.work.path().join("n.md"), "first").unwrap();
    let created = env.ok(&["save", "n.md", "--to", docs]);
    assert_eq!((created["via"].as_str(), created["replaced"].as_bool()), (Some("server"), Some(false)), "{created}");
    env.fails(5, &["save", "n.md", "--to", docs]);

    // The tag the listings give (REST) is the one WebDAV answered the write with.
    let listed = env.ok(&["ls", &n]);
    let etag = listed[0]["etag"].as_str().unwrap_or_else(|| panic!("no etag: {listed}")).to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    assert_eq!(created["etag"].as_str(), Some(etag.as_str()), "the save's tag is the listing's");
    let files = env.ok(&["files", "--in", docs]);
    let entry = files.as_array().unwrap().iter().find(|f| f["path"] == n.as_str()).unwrap_or_else(|| panic!("{files}"));
    assert_eq!(entry["etag"].as_str(), Some(etag.as_str()), "{files}");
    assert!(env.ok(&["ls", docs]).as_array().unwrap().iter().filter(|i| i["type"] == "folder").all(|i| i.get("etag").is_none()));

    // The bytes, and nothing else, on stdout (with --json too).
    for args in [vec!["get", n.as_str(), "-"], vec!["get", n.as_str(), "-", "--json"]] {
        let out = env.run(&args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.stdout, b"first", "{args:?}");
    }
    let missing = env.run(&["get", &format!("{docs}/no-such.md"), "-", "--json"]);
    assert_eq!((missing.status.code(), missing.stdout.is_empty()), (Some(4), true));

    // Over the version read: written, the old one kept, a new tag.
    std::fs::write(env.work.path().join("n.md"), "second").unwrap();
    let saved = env.ok(&["save", "n.md", "--to", docs, "--if-match", &etag]);
    assert_eq!((saved["via"].as_str(), saved["replaced"].as_bool()), (Some("server"), Some(true)), "{saved}");
    let newer = saved["etag"].as_str().unwrap_or_else(|| panic!("{saved}")).to_owned();
    assert_ne!(newer, etag);
    // Over a version that is gone: exit 5, nothing written, nothing kept.
    std::fs::write(env.work.path().join("n.md"), "third").unwrap();
    let err = env.fails(5, &["save", "n.md", "--to", docs, "--if-match", &etag]);
    assert_eq!(err["error"]["kind"], "conflict", "{err}");
    assert_eq!(env.run(&["get", &n, "-"]).stdout, b"second");
    assert_eq!(env.ok(&["versions", &n]).as_array().unwrap().len(), 1, "only the replaced version is kept");
    // The tag without its quotes works too; a file that isn't there is a conflict, not a create.
    env.ok(&["save", "n.md", "--to", docs, "--if-match", newer.trim_matches('"')]);
    assert_eq!(env.run(&["get", &n, "-"]).stdout, b"third");
    env.fails(5, &["save", "n.md", "--to", &format!("{docs}/nowhere"), "--if-match", &newer]);
    env.fails(4, &["ls", &format!("{docs}/nowhere")]);
    env.fails(2, &["save", "n.md", "--to", docs, "--if-match", &newer, "--force"]);
    env.ok(&["rm", &n]);
}

/// `server check` on CraftCloud's own server: the capabilities are known, and no probe runs
/// (docs/WEBDAV-STORAGE.md, Q3), so nothing lands in `.craftcloud/probe/`.
fn server_check_reports_known_capabilities(env: &Env) {
    let check = env.ok(&["server", "check"]);
    assert_eq!(
        (check["kind"].as_str(), check["mode"].as_str(), check["measured"].as_bool()),
        (Some("oxicloud"), Some("two_way"), Some(false)),
        "{check}"
    );
    let rows = check["checks"].as_array().unwrap_or_else(|| panic!("{check}"));
    assert!(rows.len() >= 18 && rows.iter().all(|r| r["check"].is_string() && r["result"].is_string()), "{check}");
    let own = env.ok(&["ls", ".craftcloud", "--all"]);
    assert!(!paths(&own).iter().any(|p| p == ".craftcloud/probe"), "a probe ran on OxiCloud: {own}");
}

#[test]
fn the_command_line_end_to_end() {
    let (Ok(server), Ok(user), Ok(password)) =
        (std::env::var("CRAFTCLOUD_TEST_SERVER"), std::env::var("CRAFTCLOUD_TEST_USER"), std::env::var("CRAFTCLOUD_TEST_PASSWORD"))
    else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    if kind::kind() != kind::Kind::Oxicloud {
        eprintln!("skipped: CraftCloud's own server only (the_command_line_on_another_kind_of_server runs instead)");
        return;
    }
    let env = Env { server, config: tempfile::tempdir().unwrap(), work: tempfile::tempdir().unwrap(), extra: Vec::new() };
    env.fails(3, &["ls"]);
    login(&env, &user, &password);

    let base = format!("cli-test-{}", std::process::id());
    let docs = format!("{base}/Docs");
    let (a, b, c) = (format!("{docs}/a.txt"), format!("{docs}/b.txt"), format!("{docs}/c.txt"));

    // Folders.
    env.ok(&["mkdir", "-p", &docs]);
    env.fails(5, &["mkdir", &docs]);
    env.fails(4, &["mkdir", &format!("{base}/no/such")]);

    // Upload, refuse to overwrite, overwrite with --force (keeping a version).
    std::fs::write(env.work.path().join("a.txt"), "version one").unwrap();
    let up = env.ok(&["put", "a.txt", &format!("{docs}/")]);
    assert_eq!((up["path"].as_str(), up["replaced"].as_bool()), (Some(a.as_str()), Some(false)));
    env.fails(5, &["put", "a.txt", &format!("{docs}/")]);
    std::fs::write(env.work.path().join("a.txt"), "version two!").unwrap();
    assert_eq!(env.ok(&["put", "a.txt", &a, "--force"])["replaced"], true);

    let listed = env.ok(&["ls", &docs]);
    assert_eq!(paths(&listed), std::slice::from_ref(&a));
    assert_eq!(listed[0]["size"], 12);
    assert_eq!(env.ok(&["ls", &base])[0]["type"], "folder");
    env.fails(4, &["ls", "no-such-folder"]);

    // Versions.
    let versions = env.ok(&["versions", &a]);
    assert_eq!(versions.as_array().unwrap().len(), 1);
    assert_eq!(versions[0]["size"], 11);
    // T-019: when the content was written, and when it was replaced, both with the offset.
    assert!(versions[0]["modified"].as_str().is_some_and(|t| t.len() == 25), "{versions}");
    assert!(versions[0]["modified"].as_str() <= versions[0]["replaced_at"].as_str(), "{versions}");

    // A move takes the versions along (T-050); star, recent, search.
    env.ok(&["mv", &a, &b]);
    assert_eq!(paths(&env.ok(&["ls", &docs])), std::slice::from_ref(&b));
    assert_eq!(env.ok(&["versions", &b]).as_array().unwrap().len(), 1, "the history moved along");
    env.ok(&["star", &b]);
    assert!(paths(&env.ok(&["starred"])).contains(&b));
    env.ok(&["recent"]);
    let found = env.ok(&["search", "b.txt", "--in", &base]);
    assert!(paths(&found).contains(&b), "{found}");
    env.fails(2, &["search", "x", "--type", "pdf"]);
    // CraftCloud's own folder (b.txt's kept version lives there) stays out of sight unless asked.
    let internal = |list: &Value| paths(list).iter().any(|p| p.starts_with(".craftcloud"));
    assert!(!internal(&env.ok(&["search", "b.txt"])), "versions are not search results");
    assert!(internal(&env.ok(&["search", "b.txt", "--all"])));
    assert!(!internal(&env.ok(&["ls"])));
    assert!(internal(&env.ok(&["ls", "--all"])));

    share_links(&env, &docs, &b);

    // Download and compare; a second download needs --force.
    env.ok(&["get", &b, "got.txt"]);
    assert_eq!(std::fs::read_to_string(env.work.path().join("got.txt")).unwrap(), "version two!");
    env.fails(5, &["get", &b, "got.txt"]);

    // Restore the earlier version (the history comes back along with the file).
    env.ok(&["mv", &b, &a]);
    env.ok(&["versions", &a, "restore", "1"]);
    env.ok(&["get", &a, "got.txt", "--force"]);
    assert_eq!(std::fs::read_to_string(env.work.path().join("got.txt")).unwrap(), "version one");
    assert_eq!(env.ok(&["versions", &a]).as_array().unwrap().len(), 2, "the replaced version is kept too");
    env.fails(4, &["versions", &a, "restore", "9"]);

    // Copy, trash, restore.
    env.ok(&["cp", &a, &c]);
    assert_eq!(paths(&env.ok(&["ls", &docs])), [a.clone(), c.clone()]);
    env.ok(&["rm", &c]);
    assert!(paths(&env.ok(&["trash", "ls"])).contains(&c));
    env.ok(&["trash", "restore", &c]);
    assert!(paths(&env.ok(&["ls", &docs])).contains(&c));
    restore_onto_a_taken_place(&env, &docs, &a, &c);
    a_new_file_starts_clean(&env, &docs);
    save_over_the_version_read(&env, &docs);

    // Folder download as a zip.
    env.ok(&["get", &docs]);
    assert!(Path::new(&env.work.path().join("Docs.zip")).is_file());

    // Account and server.
    assert!(env.ok(&["usage"])["used_bytes"].as_u64().is_some());
    let info = env.ok(&["server", "info"]);
    assert_eq!(info["craftcloud"], true);
    assert!(env.ok(&["server", "users", "ls"]).as_array().unwrap().iter().any(|p| p["username"] == user.as_str()));
    assert!(env.ok(&["server", "settings", "show"])["web_apps"].is_boolean());
    server_check_reports_known_capabilities(&env);
    env.fails(6, &["status"]);

    // Clean up: the test folder goes to the trash, then for good.
    env.ok(&["rm", &base]);
    env.ok(&["trash", "rm", &base]);
    env.fails(4, &["ls", &base]);
    // T-049: deleted for good, its kept versions went too.
    env.fails(4, &["ls", &format!(".craftcloud/versions/{base}"), "--all"]);
    logout_ends_the_session(&env);
}

/// Signing out ends the sign-in on the server too: its refresh token no longer works.
fn logout_ends_the_session(env: &Env) {
    let stored: Value = serde_json::from_slice(&std::fs::read(env.config.path().join("tokens.json")).unwrap()).unwrap();
    let refresh_token = stored["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(env.ok(&["logout"])["server_session_ended"], true);
    env.fails(3, &["ls"]);
    let server = ServerUrl::parse(&env.server).unwrap();
    let after = execute(&auth::refresh(&server, &refresh_token)).unwrap();
    assert!(!after.is_success(), "the refresh token still works after logout ({})", after.status);
}

/// The operator scripts (`scripts/create-owner`, `add-user`, `app-password`) against the same server: they need
/// only sh and curl (a `python3` earlier in `PATH` fails, and the test checks it never ran), and passwords with
/// `"`, `\`, `$` and letters outside ASCII reach the server as typed. It runs after the command line's test,
/// which leaves sign-ins to spare under the server's limit per minute; on a fresh server (this test alone) it
/// also creates the owner with `create-owner`.
#[cfg(unix)]
#[test]
fn the_operator_scripts() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    let (Ok(server), Ok(user), Ok(password)) =
        (std::env::var("CRAFTCLOUD_TEST_SERVER"), std::env::var("CRAFTCLOUD_TEST_USER"), std::env::var("CRAFTCLOUD_TEST_PASSWORD"))
    else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    if kind::kind() != kind::Kind::Oxicloud {
        eprintln!("skipped: the operator scripts are for CraftCloud's own server");
        return;
    }
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let fake = tempfile::tempdir().unwrap();
    // The scripts' own temporary files (mktemp, under TMPDIR) are all gone when they exit.
    let tmp = tempfile::tempdir().unwrap();
    let called = fake.path().join("python-called");
    for name in ["python3", "python"] {
        let stub = fake.path().join(name);
        std::fs::write(&stub, format!("#!/bin/sh\ntouch '{}'\nexit 97\n", called.display())).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}", fake.path().display(), std::env::var("PATH").unwrap_or_default());
    // Runs a script with only `vars` set of the variables the scripts read; waits out a 429 once.
    let run = |script: &str, args: &[&str], vars: &[(&str, &str)]| -> (Option<i32>, String, String) {
        let once = || {
            let mut cmd = Command::new(scripts.join(script));
            cmd.args(args).env("PATH", &path).env("TMPDIR", tmp.path()).stdin(Stdio::null());
            for var in
                ["CRAFTCLOUD_USER", "CRAFTCLOUD_PASSWORD", "NEW_USER", "NEW_EMAIL", "NEW_PASSWORD", "OWNER_USER", "OWNER_EMAIL", "OWNER_PASSWORD"]
            {
                cmd.env_remove(var);
            }
            let out = cmd.envs(vars.iter().copied()).output().unwrap();
            (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
        };
        let first = once();
        if !first.2.contains("error: 429") {
            return first;
        }
        std::thread::sleep(Duration::from_secs(65));
        once()
    };
    let base = ServerUrl::parse(&server).unwrap();

    // create-owner: makes the owner on a fresh server, and refuses once there is one.
    let email = format!("{user}@example.org");
    if !auth::parse_status(&execute(&auth::status(&base)).unwrap()).unwrap().initialized {
        let (code, out, err) = run("create-owner", &[&server], &[("OWNER_USER", &user), ("OWNER_EMAIL", &email), ("OWNER_PASSWORD", &password)]);
        assert_eq!(code, Some(0), "create-owner: {err}");
        assert!(out.contains(&format!("Created the owner '{user}'")), "{out}");
    }
    let (code, _, err) = run("create-owner", &[&server], &[("OWNER_USER", "x"), ("OWNER_EMAIL", "x@example.org"), ("OWNER_PASSWORD", "12345678")]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("already has an owner"), "{err}");

    // add-user: a password with JSON's special characters and more works for signing in.
    let name = format!("script-{}", std::process::id());
    let tricky = format!("pw \"q\" \\ $HOME ünï ✓ 😀 {}", std::process::id());
    let new_email = format!("{name}@example.org");
    let admin = [("CRAFTCLOUD_USER", user.as_str()), ("CRAFTCLOUD_PASSWORD", password.as_str())];
    let add = [admin[0], admin[1], ("NEW_USER", name.as_str()), ("NEW_EMAIL", new_email.as_str()), ("NEW_PASSWORD", tricky.as_str())];
    let (code, out, err) = run("add-user", &[&server], &add);
    assert_eq!(code, Some(0), "add-user: {err}");
    assert!(out.contains(&format!("Created user '{name}'")), "{out}");
    let mut signed_in = execute(&auth::login(&base, &name, &tricky)).unwrap();
    if signed_in.status == 429 {
        std::thread::sleep(Duration::from_secs(65));
        signed_in = execute(&auth::login(&base, &name, &tricky)).unwrap();
    }
    assert!(auth::parse_login(&signed_in).is_ok(), "the new user can't sign in with the password add-user sent ({})", signed_in.status);
    // …and the server's reason when it refuses.
    let (code, _, err) = run("add-user", &[&server], &add);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("The server refused (409)") && err.contains("already exists"), "{err}");

    // app-password: prints a password that opens WebDAV; the label has JSON's special characters too.
    let (code, out, err) = run("app-password", &[&server, "Label \"quoted\" \\ ü"], &admin);
    assert_eq!(code, Some(0), "app-password: {err}");
    assert!(
        out.contains(&format!("  Username:      {user}\n")) && out.contains(&format!("  WebDAV URL:    {server}/webdav/\n")),
        "app-password's output changed"
    );
    let app_password = out.lines().find_map(|l| l.strip_prefix("  App password:  ")).unwrap_or_default().to_owned();
    assert!(!app_password.is_empty(), "no app password in the output");
    // The credentials go to curl on stdin, not in its command line.
    let mut curl = Command::new("curl")
        .args(["-sS", "-K", "-", "-o", "/dev/null", "-w", "%{http_code}", "-X", "PROPFIND", "-H", "Depth: 0"])
        .arg(format!("{server}/webdav/"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut curl.stdin.take().unwrap(), format!("user = \"{user}:{app_password}\"\n").as_bytes()).unwrap();
    let dav = curl.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&dav.stdout), "207", "WebDAV with the new app password");
    let (code, _, err) = run("app-password", &[&server], &[("CRAFTCLOUD_USER", &user), ("CRAFTCLOUD_PASSWORD", "not-the-password")]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("Sign-in failed"), "{err}");

    assert!(!called.exists(), "a script ran python");
    let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(left.is_empty(), "temporary files left behind: {left:?}");
}

/// The command line on another kind of server (`CRAFTCLOUD_TEST_KIND`, with the opt-in): `login
/// --kind webdav` with the app password from `CRAFTCLOUD_APP_PASSWORD` (never an argument), `server
/// check` measuring the mode expected of the server, the file, trash and version commands through
/// the storage trait where two-way sync is allowed and refused with exit code 10 where it isn't,
/// and "not available on this server" (exit code 10, kind `unsupported`) for what only CraftCloud's
/// own server has. Skipped on CraftCloud's own server, where [`the_command_line_end_to_end`] runs.
#[test]
fn the_command_line_on_another_kind_of_server() {
    let Some(server) = kind::Server::from_env() else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    if server.kind == kind::Kind::Oxicloud {
        return;
    }
    let app_password = server.app_password();
    let env = Env {
        server: server.url.clone(),
        config: tempfile::tempdir().unwrap(),
        work: tempfile::tempdir().unwrap(),
        extra: vec![("CRAFTCLOUD_EXPERIMENTAL_WEBDAV", "1".to_owned()), ("CRAFTCLOUD_APP_PASSWORD", app_password.clone())],
    };
    env.fails(3, &["ls"]);

    // Signing in: the WebDAV address, the user name, the password from the environment; the
    // check on connect runs and the answer says the mode.
    // Nextcloud and ownCloud sign in as themselves (with an app password and `--user`: ownCloud
    // has no browser flow, and Nextcloud's Login Flow can't be driven headless; its state machine
    // is tested against a scripted server); the rest as "another WebDAV server".
    let dav = server.base().url().to_owned();
    let kind_word = match server.kind {
        kind::Kind::Nextcloud => "nextcloud",
        kind::Kind::Owncloud => "owncloud",
        _ => "webdav",
    };
    let address = if server.has_profile() { server.url.clone() } else { dav.clone() };
    let out = env.run(&["login", &address, "--kind", kind_word, "--user", &server.user, "--json"]);
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "login: {text}");
    assert!(!text.contains(&app_password), "the password never reaches the output");
    let signed: Value = serde_json::from_str(text.lines().find(|l| l.starts_with('{')).unwrap_or("null")).unwrap();
    assert_eq!(
        (signed["signed_in"].as_bool(), signed["kind"].as_str(), signed["username"].as_str(), signed["mode"].as_str()),
        (Some(true), Some(kind_word), Some(server.user.as_str()), Some(server.mode.as_str())),
        "{signed}"
    );
    assert_eq!(signed["dav_url"].as_str(), Some(dav.as_str()));

    // `server check` runs the probe afresh: measured, every step, the mode expected.
    let check = env.ok(&["server", "check"]);
    assert_eq!(
        (check["kind"].as_str(), check["mode"].as_str(), check["measured"].as_bool()),
        (Some(kind_word), Some(server.mode.as_str()), Some(true)),
        "{check}"
    );
    let server_trash =
        check["checks"].as_array().unwrap().iter().any(|c| c["check"] == "Where a delete goes" && c["result"] == "the server's own trash");
    assert_eq!(server_trash, server.has_profile(), "the server's own trash only with a profile that confirmed it: {check}");
    let steps = check["steps"].as_array().unwrap_or_else(|| panic!("{check}"));
    assert!(steps.len() >= 18 && steps.iter().all(|s| s["what"].is_string() && s["result"].is_string()), "{check}");
    assert!(!check["checks"].as_array().unwrap().is_empty(), "{check}");
    if server.mode == Mode::DownloadOnly {
        assert!(check["refused_because"].is_null(), "download only isn't a refusal: {check}");
    }

    // Not available on this server: exit code 10, kind `unsupported`, for everything only
    // CraftCloud's own server has.
    for args in [
        &["share", "ls"][..],
        &["share", "create", "x.txt"],
        &["server", "users", "ls"],
        &["server", "info"],
        &["server", "settings", "show"],
        &["starred"],
        &["star", "x.txt"],
        &["recent"],
        &["search", "x"],
        &["usage"],
        &["cp", "a.txt", "b.txt"],
    ] {
        let e = env.fails(10, args);
        assert_eq!(e["error"]["kind"].as_str(), Some("unsupported"), "{args:?}: {e}");
        assert!(e["error"]["message"].as_str().unwrap_or_default().contains("not available on this server"), "{args:?}: {e}");
    }

    let base = format!("cli-test-{}", std::process::id());
    std::fs::write(env.work.path().join("a.txt"), "version one").unwrap();
    match server.mode {
        Mode::TwoWay => file_commands_through_the_trait(&env, &base, server_trash),
        _ => writes_are_refused_under_download_only(&env, &base),
    }
    env.fails(6, &["status"]);

    // The kept versions of the test's files are CraftCloud's own leftovers in `.craftcloud/versions/`;
    // on a server with its own trash the test's folder sits there, and goes from there too. Before
    // signing out: on Nextcloud `logout` deletes the app password these requests use.
    let mut cloud = kind::Dav::new(&server);
    cloud.delete(&format!(".craftcloud/versions/{base}"), true);
    if server_trash && server.kind == kind::Kind::Nextcloud {
        let mut bin = kind::Dav::at(&server, &format!("{}/remote.php/dav/trashbin/{}/trash/", server.url, server.user));
        for entry in bin.list("").unwrap_or_default() {
            if entry.name().starts_with(&format!("{base}.d")) {
                bin.delete(&entry.path, entry.is_folder);
            }
        }
    } else if server_trash {
        // ownCloud names a trashed item by its file id; `trash ls` says where each was.
        let mut bin = kind::Dav::at(&server, &format!("{}/remote.php/dav/trash-bin/{}/", server.url, server.user));
        let listed = bin.list("").unwrap_or_default();
        for item in env.ok(&["trash", "ls"]).as_array().cloned().unwrap_or_default() {
            let (Some(path), Some(id)) = (item["path"].as_str(), item["id"].as_str()) else { continue };
            if path == base || path.starts_with(&format!("{base}/")) {
                let is_folder = listed.iter().find(|e| e.path == id).is_some_and(|e| e.is_folder);
                bin.delete(id, is_folder);
            }
        }
    }

    // Signing out forgets the app password; on Nextcloud it is deleted on the server too (ownCloud
    // has no call for it: the person revokes it in Settings › Security).
    let out = env.ok(&["logout"]);
    assert_eq!(out["server_session_ended"], server.kind == kind::Kind::Nextcloud, "{out}");
    env.fails(3, &["ls"]);
}

/// Two-way sync allowed: folders, upload (refused over an existing file, forced with a version
/// kept), listing, a move that takes the versions along, download, a version restored, the trash
/// (the server's own where its profile confirmed it, else the client's) and a restore from it, and
/// the test folder trashed as one item and then deleted for good (the client's trash only: a
/// server's own is never purged by CraftCloud). `a.txt` in the work folder holds "version one".
fn file_commands_through_the_trait(env: &Env, base: &str, server_trash: bool) {
    let docs = format!("{base}/Docs");
    let (a, b) = (format!("{docs}/a.txt"), format!("{docs}/b.txt"));
    env.ok(&["mkdir", "-p", &docs]);
    env.fails(5, &["mkdir", &docs]);
    let up = env.ok(&["put", "a.txt", &format!("{docs}/")]);
    assert_eq!((up["path"].as_str(), up["replaced"].as_bool()), (Some(a.as_str()), Some(false)));
    env.fails(5, &["put", "a.txt", &format!("{docs}/")]);
    std::fs::write(env.work.path().join("a.txt"), "version two!").unwrap();
    if server_trash {
        // Nextcloud and ownCloud name a version by its second: an overwrite within the same second
        // as the write before it leaves no version of the earlier bytes on the server
        // (docs/UPSTREAM.md E1, G1). The sync never writes a file twice in one second; this test would.
        std::thread::sleep(std::time::Duration::from_millis(1200));
    }
    assert_eq!(env.ok(&["put", "a.txt", &a, "--force"])["replaced"], true);
    let listed = env.ok(&["ls", &docs]);
    assert_eq!(paths(&listed), std::slice::from_ref(&a));
    assert_eq!(listed[0]["size"], 12);
    env.fails(4, &["ls", &format!("{base}/no-such-folder")]);
    assert_eq!(env.ok(&["versions", &a]).as_array().unwrap().len(), 1, "the kept version");

    env.ok(&["mv", &a, &b]);
    assert_eq!(paths(&env.ok(&["ls", &docs])), std::slice::from_ref(&b));
    assert_eq!(env.ok(&["versions", &b]).as_array().unwrap().len(), 1, "the history moved along");
    env.ok(&["get", &b, "got.txt"]);
    assert_eq!(std::fs::read_to_string(env.work.path().join("got.txt")).unwrap(), "version two!");
    env.fails(5, &["get", &b, "got.txt"]);
    env.ok(&["versions", &b, "restore", "1"]);
    env.ok(&["get", &b, "got.txt", "--force"]);
    assert_eq!(std::fs::read_to_string(env.work.path().join("got.txt")).unwrap(), "version one");

    env.ok(&["rm", &b]);
    assert!(paths(&env.ok(&["ls", &docs])).is_empty());
    assert!(paths(&env.ok(&["trash", "ls"])).contains(&b), "{}", env.ok(&["trash", "ls"]));
    env.ok(&["trash", "restore", &b]);
    assert_eq!(paths(&env.ok(&["ls", &docs])), std::slice::from_ref(&b));
    env.ok(&["get", &b, "got.txt", "--force"]);
    assert_eq!(std::fs::read_to_string(env.work.path().join("got.txt")).unwrap(), "version one", "restored whole");

    env.ok(&["rm", base]);
    env.fails(4, &["ls", base]);
    // The folder is in the trash (the generic driver lists a trashed folder holding exactly one
    // thing as that thing, and an entry emptied by a restore may stay behind as an empty folder:
    // docs/WEBDAV-STORAGE.md § 7.2, "As built (pull request 4a)"); `trash rm` takes each path as
    // listed, and nothing of the test is left in the trash afterwards.
    let listed = paths(&env.ok(&["trash", "ls"]));
    let inside = format!("{base}/");
    let items: Vec<&String> = listed.iter().filter(|p| *p == base || p.starts_with(&inside)).collect();
    assert!(!items.is_empty(), "{listed:?}");
    if server_trash {
        let e = env.fails(10, &["trash", "rm", items[0]]);
        assert!(e["error"]["message"].as_str().unwrap_or_default().contains("server's own trash"), "{e}");
        env.fails(10, &["trash", "empty", "--yes"]);
        return;
    }
    for item in items {
        env.ok(&["trash", "rm", item]);
    }
    assert!(!paths(&env.ok(&["trash", "ls"])).iter().any(|p| p.starts_with(base)));
}

/// Download only: reading works, and every write is refused (exit code 10, kind `unsupported`,
/// naming the mode) before anything is sent.
fn writes_are_refused_under_download_only(env: &Env, base: &str) {
    let docs = format!("{base}/Docs");
    let (a, b) = (format!("{docs}/a.txt"), format!("{docs}/b.txt"));
    env.ok(&["ls"]);
    env.ok(&["trash", "ls"]);
    for args in [&["mkdir", "-p", &docs][..], &["put", "a.txt", &a], &["rm", &a], &["mv", &a, &b], &["trash", "empty", "--yes"]] {
        let e = env.fails(10, args);
        assert_eq!(e["error"]["kind"].as_str(), Some("unsupported"), "{args:?}: {e}");
        assert!(e["error"]["message"].as_str().unwrap_or_default().contains("download only"), "{args:?}: {e}");
    }
    assert!(!paths(&env.ok(&["ls"])).iter().any(|p| p.starts_with(base)), "nothing was made");
}
