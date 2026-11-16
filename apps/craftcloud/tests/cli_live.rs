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

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};

use craftcloud_core::ServerUrl;
use craftcloud_core::api::auth;
use craftcloud_native::http::execute;
use serde_json::Value;

struct Env {
    server: String,
    config: tempfile::TempDir,
    work: tempfile::TempDir,
}

impl Env {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_craftcloud"));
        c.args(args)
            .env("CRAFTCLOUD_CONFIG_DIR", self.config.path())
            .env("CRAFTCLOUD_KEYCHAIN", "off")
            .env_remove("CRAFTCLOUD_SERVER")
            .current_dir(self.work.path())
            .stdin(Stdio::null());
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

#[test]
fn the_command_line_end_to_end() {
    let (Ok(server), Ok(user), Ok(password)) =
        (std::env::var("CRAFTCLOUD_TEST_SERVER"), std::env::var("CRAFTCLOUD_TEST_USER"), std::env::var("CRAFTCLOUD_TEST_PASSWORD"))
    else {
        eprintln!("skipped: CRAFTCLOUD_TEST_SERVER not set");
        return;
    };
    let env = Env { server, config: tempfile::tempdir().unwrap(), work: tempfile::tempdir().unwrap() };
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

    // Folder download as a zip.
    env.ok(&["get", &docs]);
    assert!(Path::new(&env.work.path().join("Docs.zip")).is_file());

    // Account and server.
    assert!(env.ok(&["usage"])["used_bytes"].as_u64().is_some());
    let info = env.ok(&["server", "info"]);
    assert_eq!(info["craftcloud"], true);
    assert!(env.ok(&["server", "users", "ls"]).as_array().unwrap().iter().any(|p| p["username"] == user.as_str()));
    assert!(env.ok(&["server", "settings", "show"])["web_apps"].is_boolean());
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
