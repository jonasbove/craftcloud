//! Argument parsing: what each command line means, and that it never starts the desktop client
//! by accident.

use clap::{CommandFactory, Parser};

use super::{Cli, Cmd, ServerCmd, SettingsCmd, ShareCmd, TrashCmd, UsersCmd};

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("craftcloud").chain(args.iter().copied())).unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

fn fails(args: &[&str]) -> clap::error::ErrorKind {
    Cli::try_parse_from(std::iter::once("craftcloud").chain(args.iter().copied())).map(|_| ()).unwrap_err().kind()
}

#[test]
fn the_definition_is_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn no_command_starts_the_desktop_client() {
    assert!(parse(&[]).command.is_none());
    let hidden = parse(&["--hidden"]);
    assert!(hidden.hidden && hidden.command.is_none());
}

#[test]
fn global_options_go_anywhere() {
    let cli = parse(&["ls", "Documents", "--json", "--server", "cloud.example.org"]);
    assert!(cli.global.json);
    assert_eq!(cli.global.server.as_deref(), Some("cloud.example.org"));
    assert!(matches!(cli.command, Some(Cmd::Ls { path, all: false, app: None }) if path == "Documents"));
    assert!(matches!(parse(&["ls"]).command, Some(Cmd::Ls { path, .. }) if path.is_empty()));
    assert!(matches!(parse(&["search", "x", "--all"]).command, Some(Cmd::Search { all: true, .. })));
    assert!(matches!(parse(&["trash", "ls", "--all"]).command, Some(Cmd::Trash(TrashCmd::Ls { all: true }))));
}

#[test]
fn file_commands() {
    assert!(
        matches!(parse(&["get", "a.txt", "-", "-f"]).command, Some(Cmd::Get { remote, local: Some(l), force: true }) if remote == "a.txt" && l == "-")
    );
    assert!(matches!(parse(&["put", "a.txt"]).command, Some(Cmd::Put { remote: None, force: false, .. })));
    assert!(matches!(parse(&["mkdir", "-p", "a/b"]).command, Some(Cmd::Mkdir { parents: true, .. })));
    assert!(matches!(parse(&["rm", "a", "b"]).command, Some(Cmd::Rm { paths }) if paths.len() == 2));
    assert_eq!(fails(&["rm"]), clap::error::ErrorKind::MissingRequiredArgument);
    assert!(
        matches!(parse(&["trash", "restore", "a.txt"]).command, Some(Cmd::Trash(TrashCmd::Restore { item, to: None, rename: false })) if item == "a.txt")
    );
    assert!(
        matches!(parse(&["trash", "restore", "a.txt", "--as", "b.txt"]).command, Some(Cmd::Trash(TrashCmd::Restore { to: Some(t), .. })) if t == "b.txt")
    );
    assert!(matches!(parse(&["trash", "restore", "a.txt", "--rename"]).command, Some(Cmd::Trash(TrashCmd::Restore { rename: true, .. }))));
    assert_eq!(fails(&["trash", "restore", "a.txt", "--rename", "--as", "b"]), clap::error::ErrorKind::ArgumentConflict);
    assert!(matches!(parse(&["trash", "empty", "--yes"]).command, Some(Cmd::Trash(TrashCmd::Empty { yes: true }))));
    assert!(matches!(
        parse(&["search", "q3", "--type", "document", "--in", "Docs"]).command,
        Some(Cmd::Search { kind: Some(_), within: Some(_), limit: 50, .. })
    ));
}

#[test]
fn craft_app_commands() {
    assert!(matches!(parse(&["apps", "--json"]).command, Some(Cmd::Apps)));
    assert!(matches!(
        parse(&["files", "--app", "wordcraft", "--in", "Documents"]).command,
        Some(Cmd::Files { app: Some(a), within: Some(w) }) if a == "wordcraft" && w == "Documents"
    ));
    assert!(matches!(parse(&["files"]).command, Some(Cmd::Files { app: None, within: None })));
    assert!(matches!(
        parse(&["save", "notes.md", "--to", "Documents", "--wait", "-f"]).command,
        Some(Cmd::Save { local, to: Some(t), app: None, force: true, wait: true }) if local == "notes.md" && t == "Documents"
    ));
    assert!(matches!(parse(&["ls", "--app", "gridcraft"]).command, Some(Cmd::Ls { app: Some(_), .. })));
    assert!(matches!(parse(&["recent", "--app", "wordcraft"]).command, Some(Cmd::Recent { app: Some(_), clear: false, .. })));
    assert!(matches!(parse(&["starred", "--app", "pdfcraft"]).command, Some(Cmd::Starred { app: Some(_), .. })));
    assert!(matches!(parse(&["search", "q3", "--app", "deckcraft"]).command, Some(Cmd::Search { app: Some(_), .. })));
    assert_eq!(fails(&["recent", "--clear", "--app", "wordcraft"]), clap::error::ErrorKind::ArgumentConflict);
    assert_eq!(fails(&["save"]), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn sync_modes() {
    assert!(matches!(parse(&["sync"]).command, Some(Cmd::Sync { no_wait: false, once: false, confirm_deletes: false, discard_deletes: false })));
    assert!(matches!(parse(&["sync", "--confirm-deletes"]).command, Some(Cmd::Sync { confirm_deletes: true, .. })));
    assert!(matches!(parse(&["sync", "--once", "--discard-deletes"]).command, Some(Cmd::Sync { once: true, discard_deletes: true, .. })));
    assert_eq!(fails(&["sync", "--confirm-deletes", "--discard-deletes"]), clap::error::ErrorKind::ArgumentConflict);
    assert!(matches!(parse(&["sync", "--once"]).command, Some(Cmd::Sync { once: true, .. })));
    assert_eq!(fails(&["sync", "--once", "--no-wait"]), clap::error::ErrorKind::ArgumentConflict);
    assert!(matches!(parse(&["daemon", "--json"]).command, Some(Cmd::Daemon)));
}

#[test]
fn versions_take_an_optional_restore() {
    assert!(matches!(parse(&["versions", "a.md"]).command, Some(Cmd::Versions { action: None, n: None, .. })));
    assert!(matches!(parse(&["versions", "a.md", "restore", "2"]).command, Some(Cmd::Versions { action: Some(_), n: Some(2), .. })));
    assert_eq!(fails(&["versions", "a.md", "undo", "2"]), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn share_and_server_commands() {
    assert!(matches!(
        parse(&["share", "create", "a.pdf", "--expires", "7d", "--password"]).command,
        Some(Cmd::Share(ShareCmd::Create { expires: Some(_), password: true, .. }))
    ));
    assert!(matches!(parse(&["share", "ls"]).command, Some(Cmd::Share(ShareCmd::Ls { path: None }))));
    assert!(matches!(
        parse(&["server", "users", "add", "jane", "--email", "jane@example.org"]).command,
        Some(Cmd::Server(ServerCmd::Users(UsersCmd::Add { .. })))
    ));
    assert!(matches!(
        parse(&["server", "settings", "set", "--web-apps", "on", "--disable-app", "a", "--disable-app", "b"]).command,
        Some(Cmd::Server(ServerCmd::Settings(SettingsCmd::Set { web_apps: Some(_), disable_app, .. }))) if disable_app.len() == 2
    ));
    assert_eq!(fails(&["server", "settings", "set", "--web-apps", "maybe"]), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn help_is_there_with_examples() {
    let help = Cli::command().render_long_help().to_string();
    assert!(help.contains("Examples:") && help.contains("Exit codes"), "{help}");
    assert!(!help.contains("--hidden"), "the login-start flag stays out of the help");
    assert_eq!(fails(&["--help"]), clap::error::ErrorKind::DisplayHelp);
    assert_eq!(fails(&["nope"]), clap::error::ErrorKind::InvalidSubcommand);
    assert!(help.contains("9 sync held"), "every exit code is in the help: {help}");
}

/// Every command has a one-line summary and an example, and every argument says what it is.
#[test]
fn every_command_has_a_summary_an_example_and_documented_arguments() {
    fn walk(cmd: &clap::Command, path: &str, missing: &mut Vec<String>) {
        let path = format!("{path} {}", cmd.get_name()).trim().to_owned();
        let leaves: Vec<&clap::Command> = cmd.get_subcommands().filter(|c| c.get_name() != "help").collect();
        if cmd.get_about().is_none() {
            missing.push(format!("{path}: no summary"));
        }
        for arg in cmd.get_arguments().filter(|a| !a.is_hide_set() && !a.is_global_set()) {
            if arg.get_help().is_none() && !matches!(arg.get_id().as_str(), "help" | "version") {
                missing.push(format!("{path}: {} has no help", arg.get_id()));
            }
        }
        if leaves.is_empty() {
            if !cmd.get_after_help().is_some_and(|h| h.to_string().contains("Example")) {
                missing.push(format!("{path}: no example"));
            }
        } else {
            for sub in leaves {
                walk(sub, &path, missing);
            }
        }
    }
    let root = Cli::command();
    let mut missing = Vec::new();
    for sub in root.get_subcommands().filter(|c| c.get_name() != "help") {
        walk(sub, "", &mut missing);
    }
    assert!(missing.is_empty(), "{missing:#?}");
}

#[test]
fn a_link_is_not_a_command() {
    for link in ["craftcloud://open?path=a.md", "CraftCloud:reveal?path=a.md"] {
        assert!(super::is_link(link), "{link}");
    }
    for arg in ["craftcloud", "ls", "--json", "https://cloud.example.org/", ""] {
        assert!(!super::is_link(arg), "{arg}");
    }
}

/// T-026: a link after the global options is still a link, not an unknown command.
#[test]
fn a_link_is_found_after_global_options() {
    let link = "craftcloud://open?path=a.md";
    let found = |args: &[&str]| super::link_arg(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>()).map(str::to_owned);
    assert_eq!(found(&[link]).as_deref(), Some(link));
    assert_eq!(found(&["--json", link]).as_deref(), Some(link));
    assert_eq!(found(&["--server", "cloud.example.org", link, "--json"]).as_deref(), Some(link));
    assert_eq!(found(&["--server=cloud.example.org", "--json", link]).as_deref(), Some(link));
    assert_eq!(found(&["ls", link]), None, "an argument of a command stays one");
    assert_eq!(found(&["--json", "ls"]), None);
    assert_eq!(found(&["--server"]), None);
    assert_eq!(found(&[]), None);
}

#[test]
fn a_closed_stdout_pipe_ends_quietly() {
    assert!(super::is_closed_pipe("failed printing to stdout: Broken pipe (os error 32)"));
    assert!(!super::is_closed_pipe("failed printing to stdout: No space left on device (os error 28)"));
    assert!(!super::is_closed_pipe("index out of bounds"));
}

#[test]
fn a_held_sync_is_its_own_exit_code_and_says_what_to_run() {
    use craftcloud_native::control::Status;
    let held = Status { state: "mass_delete".into(), pending_deletes: vec!["Old/a.txt".into(), "Old/b.txt".into()], ..Status::default() };
    let e = super::account::failed(&held).unwrap_err();
    assert_eq!((e.kind, e.kind.code()), (super::output::Fail::Held, 9));
    assert!(e.message.contains("delete 2 files") && e.message.contains("craftcloud sync --confirm-deletes"), "{}", e.message);
    assert!(super::account::failed(&Status { state: "up_to_date".into(), ..Status::default() }).is_ok());
}

/// #15: a file still being saved after `sync --once`'s second pass is said, in the headline and the
/// JSON, without the exit code of a hold (it waits for time, not for the person).
#[test]
fn a_file_being_saved_is_said() {
    use craftcloud_core::sync::{SyncProgress, SyncState};
    use craftcloud_native::control::Status;
    let p = SyncProgress { state: SyncState::UpToDate, waiting: vec!["Documents/notes.md".into()], ..SyncProgress::default() };
    let s = Status::from_progress(&p, "https://cloud.example.org", "/home/john/CraftCloud");
    assert_eq!(super::account::state_words(&s), "Up to date · 1 file is still being saved: it syncs in the next pass.");
    assert_eq!(serde_json::to_value(&s).unwrap()["waiting"], serde_json::json!(["Documents/notes.md"]));
    assert!(super::account::failed(&s).is_ok());
    let none = Status::from_progress(&SyncProgress { state: SyncState::UpToDate, ..SyncProgress::default() }, "", "");
    assert!(serde_json::to_value(&none).unwrap().get("waiting").is_none(), "left out when nothing waits");
}

/// T-049: storage use says how much the kept versions take, in the same (decimal) units; a file
/// that doesn't fit is said in the headline, without changing the exit code.
#[test]
fn usage_counts_versions_and_no_room_is_said() {
    use craftcloud_native::control::Status;
    assert_eq!(super::account::usage_words(1_100_000_000, 10_000_000_000, 200_000_000), "1.1 GB of 10 GB used (200 MB in earlier versions)");
    assert_eq!(super::account::usage_words(8_000_000, 0, 0), "8.0 MB of no limit used");
    let full = Status { state: "up_to_date".into(), no_room: vec!["Video/big.mov".into()], ..Status::default() };
    assert!(super::account::state_words(&full).starts_with("Up to date · 1 not synced (no room"), "{}", super::account::state_words(&full));
    assert!(super::account::held(&full).is_ok() && super::account::failed(&full).is_ok());
    assert_eq!(serde_json::to_value(&full).unwrap()["no_room"], serde_json::json!(["Video/big.mov"]));
}

/// T-042: confirming (or discarding) a held mass delete says what it does.
#[test]
fn held_deletes_say_what_happens() {
    use super::account::{Deletes, deletes_words};
    assert_eq!(deletes_words(Deletes::Confirm, 3000, false), "Deleting 3000 files…");
    assert_eq!(deletes_words(Deletes::Confirm, 3000, true), "Deleted 3000 files.");
    assert_eq!(deletes_words(Deletes::Discard, 1, true), "Brought back 1 file.");
    assert_eq!(deletes_words(Deletes::Confirm, 0, false), "Deleting the held files…");
}

/// T-033: a place that can't be read holds the sync (code 9, and the headline says so); a
/// symbolic link or a name the cloud can't take is left out for good and doesn't.
#[test]
fn an_unreadable_place_is_held_and_a_symlink_is_not() {
    use craftcloud_native::control::{NotSynced, Status};
    let place = |path: &str, reason: &str| NotSynced { path: path.into(), reason: reason.into() };
    let informational =
        Status { state: "up_to_date".into(), not_synced: vec![place("t13/link", "symlink"), place("t13/a\\b", "name")], ..Status::default() };
    assert!(super::account::held(&informational).is_ok() && super::account::failed(&informational).is_ok());
    assert_eq!(super::account::state_words(&informational), "Up to date.");
    let locked = Status { not_synced: vec![place("t13/locked", "unreadable"), place("t13/link", "symlink")], ..informational };
    let e = super::account::held(&locked).unwrap_err();
    assert_eq!(e.kind.code(), 9);
    assert!(e.message.starts_with("Up to date · 1 place can't be read"), "{}", e.message);
    assert_eq!(super::account::failed(&locked).unwrap_err().kind.code(), 9, "sync through the client says the same");
    let json = serde_json::to_value(&locked).unwrap();
    assert_eq!(json["not_synced"][0], serde_json::json!({"path": "t13/locked", "reason": "unreadable"}));
    assert_eq!(serde_json::from_value::<Status>(json).unwrap(), locked, "read back");
    // A client started before reasons were said answers bare paths: still read (cautiously).
    let old: Vec<NotSynced> = serde_json::from_str(r#"["t13/locked"]"#).unwrap();
    assert!(old[0].unreadable() && old[0].path == "t13/locked");
}

/// `status` exits with code 9 while the sync folder is missing, as the engine reports it.
#[test]
fn a_missing_sync_folder_is_held() {
    use craftcloud_core::sync::{SyncProgress, SyncState};
    use craftcloud_native::control::Status;
    let p = SyncProgress { state: SyncState::FolderMissing, ..SyncProgress::default() };
    let s = Status::from_progress(&p, "https://cloud.example.org", "/home/john/CraftCloud");
    assert_eq!(s.state, "folder_missing");
    let e = super::account::held(&s).unwrap_err();
    assert_eq!(e.kind.code(), 9);
    assert!(e.message.starts_with("Held: the sync folder is missing"), "{}", e.message);
    assert_eq!(super::account::failed(&s).unwrap_err().kind.code(), 9, "sync through the client says the same");
}

/// T-053: a single file that can't be read is reported like a folder: code 9, the headline, the
/// `not synced` line and the `{path, reason}` entry.
#[test]
fn an_unreadable_file_is_held_like_a_folder() {
    use craftcloud_native::control::{NotSynced, Status};
    let locked = NotSynced { path: "t13/locked.txt".into(), reason: "unreadable".into() };
    let s = Status { state: "up_to_date".into(), not_synced: vec![locked.clone()], ..Status::default() };
    let e = super::account::held(&s).unwrap_err();
    assert_eq!(e.kind.code(), 9);
    assert!(e.message.starts_with("Up to date · 1 place can't be read"), "{}", e.message);
    assert_eq!(super::account::not_synced_line(&locked), "not synced t13/locked.txt  (can't be read)");
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["not_synced"][0], serde_json::json!({"path": "t13/locked.txt", "reason": "unreadable"}));
}

#[test]
fn files_that_failed_to_sync_are_in_the_headline() {
    use craftcloud_native::control::Status;
    let s = Status { state: "up_to_date".into(), failed: vec!["Docs/a very long name.txt".into()], ..Status::default() };
    let words = super::account::state_words(&s);
    assert!(words.starts_with("Up to date · 1 file couldn't sync"), "{words}");
    assert_eq!(super::account::state_words(&Status { state: "up_to_date".into(), ..Status::default() }), "Up to date.");
}

/// On Windows `craftcloud.com` relays the console through a pipe and says so, so the questions
/// (passwords, "Delete …? [y/N]") are still asked (#4). Any other value, or none, is a pipe.
#[test]
fn a_relayed_console_counts_as_a_terminal() {
    use std::ffi::OsStr;
    assert!(super::typed_input(true, None));
    assert!(super::typed_input(false, Some(OsStr::new("1"))));
    assert!(!super::typed_input(false, None));
    assert!(!super::typed_input(false, Some(OsStr::new(""))));
    assert!(!super::typed_input(false, Some(OsStr::new("0"))));
}
