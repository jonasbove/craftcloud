//! The opt-in for other kinds of server until 0.3.0 (docs/WEBDAV-STORAGE.md § 16), on the built
//! program: without `CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1` (unset, or any other value) `--kind webdav`
//! is a usage error and a settings file naming another kind of server is refused with the one
//! message, nothing written; with it, the same commands get past the check (and fail further on,
//! since nothing listens on the address they name).

use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const REFUSED: &str = "This build syncs only with a CraftCloud server; set CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 to try other servers";

/// Runs `craftcloud <args> --json` with the settings in `config` and the variable set to `opt_in`
/// (unset for `None`); tokens in a file, never the keychain.
fn run(config: &Path, opt_in: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_craftcloud"));
    command
        .args(args)
        .arg("--json")
        .env("CRAFTCLOUD_CONFIG_DIR", config)
        .env("CRAFTCLOUD_KEYCHAIN", "off")
        .env("CRAFTCLOUD_APP_PASSWORD", "not-a-real-password")
        .env_remove("CRAFTCLOUD_SERVER")
        .stdin(Stdio::null());
    match opt_in {
        Some(value) => command.env("CRAFTCLOUD_EXPERIMENTAL_WEBDAV", value),
        None => command.env_remove("CRAFTCLOUD_EXPERIMENTAL_WEBDAV"),
    };
    command.output().unwrap()
}

/// The JSON error on stderr.
fn error(out: &Output) -> Value {
    let text = String::from_utf8_lossy(&out.stderr);
    serde_json::from_str(text.lines().last().unwrap_or("null")).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// A settings folder whose file names another WebDAV server (left from a run with the opt-in),
/// and the sync folder it names; both with nothing else in them.
fn webdav_settings() -> (tempfile::TempDir, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join("config.toml"),
        format!(
            "schema_version = 1\nserver = \"http://127.0.0.1:1\"\nlocal_folder = {:?}\nstorage = \"webdav\"\ndav_url = \"http://127.0.0.1:1/remote.php/dav/files/john/\"\n",
            folder.path().display().to_string()
        ),
    )
    .unwrap();
    (config, folder)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// The values that don't switch the other kinds on: unset, and anything but the exact `1`.
const OFF: [Option<&str>; 4] = [None, Some("0"), Some("true"), Some("yes")];

#[test]
fn without_the_opt_in_another_kind_of_server_is_refused_as_a_usage_error_on_login() {
    for opt_in in OFF {
        let config = tempfile::tempdir().unwrap();
        let out = run(config.path(), opt_in, &["login", "http://127.0.0.1:1/remote.php/dav/files/john/", "--kind", "webdav", "--user", "john"]);
        assert_eq!(out.status.code(), Some(2), "{opt_in:?}: {}", String::from_utf8_lossy(&out.stderr));
        let e = error(&out);
        assert_eq!((e["error"]["kind"].as_str(), e["error"]["message"].as_str()), (Some("usage"), Some(REFUSED)), "{opt_in:?}: {e}");
        assert_eq!(names(config.path()), Vec::<String>::new(), "{opt_in:?}: nothing written");
        // Every other kind is a usage error too; CraftCloud's own goes on as before (to the browser
        // sign-in, which fails here since nothing listens: 8, not 2).
        for kind in ["nextcloud", "owncloud", "opencloud", "dropbox"] {
            assert_eq!(run(config.path(), opt_in, &["login", "http://127.0.0.1:1", "--kind", kind]).status.code(), Some(2), "{opt_in:?} {kind}");
        }
        assert_eq!(run(config.path(), opt_in, &["login", "http://127.0.0.1:1", "--kind", "oxicloud"]).status.code(), Some(8), "{opt_in:?}");
    }
}

#[test]
fn without_the_opt_in_a_settings_file_naming_another_kind_is_refused_and_nothing_is_written() {
    for opt_in in OFF {
        let (config, folder) = webdav_settings();
        let before = (names(config.path()), names(folder.path()));
        for args in [&["status"][..], &["sync", "--once"], &["ls"], &["ls", "Documents"], &["server", "check"], &["put", "Cargo.toml"]] {
            let out = run(config.path(), opt_in, args);
            assert_eq!(out.status.code(), Some(1), "{opt_in:?} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            let e = error(&out);
            assert_eq!((e["error"]["kind"].as_str(), e["error"]["message"].as_str()), (Some("error"), Some(REFUSED)), "{opt_in:?} {args:?}: {e}");
            assert!(out.stdout.is_empty(), "{opt_in:?} {args:?}: nothing on stdout: {}", String::from_utf8_lossy(&out.stdout));
        }
        assert_eq!((names(config.path()), names(folder.path())), before, "{opt_in:?}: nothing written anywhere");
        // Signing out still works, and makes the settings CraftCloud's own server's again.
        let out = run(config.path(), opt_in, &["logout"]);
        assert_eq!(out.status.code(), Some(0), "{opt_in:?}: {}", String::from_utf8_lossy(&out.stderr));
        let file = std::fs::read_to_string(config.path().join("config.toml")).unwrap();
        assert!(file.contains("storage = \"oxicloud\""), "{opt_in:?}: {file}");
    }
}

#[test]
fn without_the_opt_in_kind_and_user_stay_out_of_the_help() {
    for opt_in in OFF {
        let out = run(tempfile::tempdir().unwrap().path(), opt_in, &["login", "--help"]);
        let help = String::from_utf8_lossy(&out.stdout);
        assert!(!help.contains("--kind") && !help.contains("--user"), "{opt_in:?}: {help}");
    }
    let out = run(tempfile::tempdir().unwrap().path(), Some("1"), &["login", "--help"]);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--kind") && help.contains("--user") && help.contains("webdav"), "{help}");
}

#[test]
fn with_the_opt_in_the_same_commands_get_past_the_check() {
    let (config, _folder) = webdav_settings();
    // Nothing listens on the address, so each fails further on, for its own reason: not with the
    // refusal, and never as a usage error.
    let expected = [
        (&["status"][..], 6, "no_client"),
        (&["ls"], 3, "signed_out"),
        (&["sync", "--once"], 3, "signed_out"),
        (&["server", "check"], 3, "signed_out"),
    ];
    for (args, code, kind) in expected {
        let out = run(config.path(), Some("1"), args);
        let e = error(&out);
        assert_eq!(out.status.code(), Some(code), "{args:?}: {e}");
        assert_eq!(e["error"]["kind"].as_str(), Some(kind), "{args:?}: {e}");
        assert!(!e["error"]["message"].as_str().unwrap_or_default().contains("CRAFTCLOUD_EXPERIMENTAL_WEBDAV"), "{args:?}: {e}");
    }
    // `login --kind webdav` reaches the server (which isn't there: 8), its password from the
    // environment, never an argument.
    let fresh = tempfile::tempdir().unwrap();
    let out = run(fresh.path(), Some("1"), &["login", "http://127.0.0.1:1/remote.php/dav/files/john/", "--kind", "webdav", "--user", "john"]);
    let e = error(&out);
    assert_eq!((out.status.code(), e["error"]["kind"].as_str()), (Some(8), Some("unreachable")), "{e}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("not-a-real-password"), "the password never reaches the output");
    assert_eq!(names(fresh.path()), Vec::<String>::new(), "a sign-in that didn't work saved nothing");
    // Without --user, or with a kind that has no driver yet, a usage error that says so.
    let out = run(fresh.path(), Some("1"), &["login", "http://127.0.0.1:1/remote.php/dav/files/john/", "--kind", "webdav"]);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(error(&out)["error"]["message"].as_str().unwrap().contains("--user"));
    let out = run(fresh.path(), Some("1"), &["login", "http://127.0.0.1:1", "--kind", "nextcloud", "--user", "john"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(error(&out)["error"]["message"].as_str().unwrap().contains("can't sync with this kind of server yet"));
    // A password over plain http to another computer is refused before anything is sent.
    let out = run(fresh.path(), Some("1"), &["login", "http://192.0.2.1/dav/john/", "--kind", "webdav", "--user", "john"]);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(error(&out)["error"]["message"].as_str().unwrap().contains("isn't https"));
}
