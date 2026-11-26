//! `craftcloud <command>`: the desktop client's command line. With no command (or only
//! `--hidden`) the program starts the desktop client as before; with one it runs it and exits,
//! never opening a window. Reference: docs/CLI.md.

mod account;
mod admin;
mod check;
mod craft;
mod files;
mod headless;
mod output;
mod remote;
mod sharing;
mod webdav;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use self::output::{CliError, Result};
use self::remote::Ctx;

const EXAMPLES: &str = "\
Examples:
  craftcloud login cloud.example.org      sign in through the browser
  craftcloud ls Documents                 list a folder
  craftcloud put report.pdf Documents/    upload into a folder
  craftcloud get Documents/report.pdf     download into the current folder
  craftcloud share create Documents/report.pdf --expires 7d
  craftcloud status --json                the desktop client's sync state, for scripts

Paths are cloud paths, from the top of your cloud: Documents/report.pdf.
Exit codes: 0 ok, 1 error, 2 usage, 3 not signed in, 4 not found, 5 conflict (already
there, changed meanwhile, or another sync engine has the folder), 6 desktop client not
running, 7 not allowed, 8 server unreachable, 9 sync held (waits for you), 10 not available
on this server.
Every command takes --json (output for scripts) and --server URL (or CRAFTCLOUD_SERVER).";

#[derive(Debug, Parser)]
#[command(name = "craftcloud", version, about = "CraftCloud: your own cloud for the Craft apps.", after_help = EXAMPLES)]
pub struct Cli {
    /// Start the desktop client in the tray, without its window (used at login).
    #[arg(long, hide = true)]
    pub hidden: bool,
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

#[derive(Debug, Args)]
pub struct Global {
    /// The server to use, instead of the desktop client's.
    #[arg(long, global = true, env = "CRAFTCLOUD_SERVER", value_name = "URL")]
    pub server: Option<String>,
    /// Print machine-readable JSON (errors go to stderr as JSON too).
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Sign this computer in through the browser (the same sign-in as the desktop client).
    #[command(after_help = "Example:\n  craftcloud login cloud.example.org")]
    Login {
        /// The server's address; without it, the desktop client's server.
        server: Option<String>,
        /// Print the address to open instead of opening the browser.
        #[arg(long)]
        no_browser: bool,
        /// The kind of server: oxicloud (CraftCloud's own, the default), nextcloud (signed in
        /// through the browser, Nextcloud's own sign-in; SERVER is its address, e.g.
        /// cloud.example.org; detected when left out), owncloud (an app password you made in the
        /// server's Settings › Security, with --user; SERVER is its address; detected when left
        /// out) or webdav (any other WebDAV server, signed in to with an app or WebDAV password;
        /// SERVER is then its WebDAV address, e.g. https://cloud.example.org/remote.php/dav/files/NAME/).
        /// Experimental: needs CRAFTCLOUD_EXPERIMENTAL_WEBDAV=1 until 0.3.0.
        #[arg(long, value_name = "KIND", hide = !craftcloud_native::experimental::experimental_webdav())]
        kind: Option<String>,
        /// The user name, for a server signed in to with an app or WebDAV password: required with
        /// --kind webdav and --kind owncloud; with --kind nextcloud it signs in with an app
        /// password you made in the server's Settings › Security instead of the browser.
        #[arg(long, value_name = "NAME", hide = !craftcloud_native::experimental::experimental_webdav())]
        user: Option<String>,
    },
    /// Sign this computer out (the desktop client too).
    #[command(after_help = "Example:\n  craftcloud logout")]
    Logout,
    /// Show the running desktop client's sync state.
    #[command(after_help = "Example:\n  craftcloud status --json")]
    Status,
    /// Sync now: through the running desktop client (or daemon), or, if none runs, one sync pass here.
    #[command(
        after_help = "Examples:\n  craftcloud sync\n  craftcloud sync --no-wait\n  craftcloud sync --once --json      one pass here, a summary of what moved\n  craftcloud sync --confirm-deletes  carry out a held mass delete"
    )]
    Sync {
        /// Ask the desktop client to sync and return at once.
        #[arg(long, conflicts_with = "once")]
        no_wait: bool,
        /// Run one full sync pass in this process and exit (refused while another one syncs the folder).
        #[arg(long)]
        once: bool,
        /// Carry out the deletions a mass-delete hold waits on.
        #[arg(long, conflicts_with_all = ["discard_deletes", "no_wait"])]
        confirm_deletes: bool,
        /// Bring back the files a mass-delete hold would delete, instead of deleting them.
        #[arg(long, conflicts_with = "no_wait")]
        discard_deletes: bool,
    },
    /// Sync without a window or tray until stopped (Ctrl+C), answering status/sync/pause/resume.
    #[command(after_help = "Example:\n  craftcloud daemon 2>> ~/craftcloud.log")]
    Daemon,
    /// Pause the desktop client's syncing.
    #[command(after_help = "Example:\n  craftcloud pause")]
    Pause,
    /// Resume the desktop client's syncing.
    #[command(after_help = "Example:\n  craftcloud resume")]
    Resume,
    /// List a folder.
    #[command(after_help = "Examples:\n  craftcloud ls\n  craftcloud ls Documents/Reports\n  craftcloud ls Documents --app wordcraft --json")]
    Ls {
        /// The folder (default: the top of the cloud).
        #[arg(default_value = "")]
        path: String,
        /// Also show CraftCloud's own `.craftcloud` folder (kept versions).
        #[arg(long)]
        all: bool,
        /// Only files this Craft app opens (see `craftcloud apps`).
        #[arg(long, value_name = "APP")]
        app: Option<String>,
    },
    /// The Craft apps and the file types each opens.
    #[command(after_help = "Example:\n  craftcloud apps --json")]
    Apps,
    /// The synced files on this computer (from the running desktop client, offline), or, with no
    /// client running, all files in the cloud.
    #[command(after_help = "Examples:\n  craftcloud files --app wordcraft --json\n  craftcloud files --app gridcraft --in Documents")]
    Files {
        /// Only files this Craft app opens (see `craftcloud apps`).
        #[arg(long, value_name = "APP")]
        app: Option<String>,
        /// Only inside this folder.
        #[arg(long = "in", value_name = "FOLDER")]
        within: Option<String>,
    },
    /// Save a file into CraftCloud: into the sync folder for the desktop client to upload, or,
    /// with no client running, straight to the server.
    #[command(
        after_help = "Examples:\n  craftcloud save notes.md                   into WordCraft/ (the app that opens it)\n  craftcloud save q3.xlsx --to Documents/Q3 --wait --json\n  craftcloud save logo.png --app figcraft --force\n  craftcloud save notes.md --to Documents --if-match '\"9ff164e6f5312e5a-1794606820\"' --json\n\nWithout --force or --if-match a save never replaces a file that is there (exit code 5)."
    )]
    Save {
        /// The local file to save.
        local: String,
        /// The cloud folder to save into (default: a folder named after the app, e.g. WordCraft).
        #[arg(long, value_name = "FOLDER")]
        to: Option<String>,
        /// The app whose folder it goes into (default: the app that opens the file).
        #[arg(long, value_name = "APP")]
        app: Option<String>,
        /// Replace a file that is already there (its old version is kept, see `versions`).
        #[arg(long, short)]
        force: bool,
        /// Replace the cloud file only if it is still this version (its `etag` from `files --json`
        /// or `ls --json`); else exit code 5 and nothing is written. Goes straight to the server,
        /// even with the desktop client running; the old version is kept.
        #[arg(long, value_name = "ETAG", conflicts_with = "force")]
        if_match: Option<String>,
        /// Wait until the desktop client has uploaded it.
        #[arg(long)]
        wait: bool,
    },
    /// Download a file (a folder downloads as a .zip).
    #[command(
        after_help = "Examples:\n  craftcloud get Documents/report.pdf\n  craftcloud get Documents/report.pdf ~/Desktop/\n  craftcloud get Documents/report.pdf - | less"
    )]
    Get {
        /// The cloud file or folder.
        remote: String,
        /// File or folder to save to (default: the current folder); `-` writes to stdout.
        local: Option<String>,
        /// Replace a local file that is already there.
        #[arg(long, short)]
        force: bool,
    },
    /// Upload a file.
    #[command(
        after_help = "Examples:\n  craftcloud put report.pdf                 to the top of the cloud\n  craftcloud put report.pdf Documents/      into a folder\n  craftcloud put report.pdf Documents/q3.pdf --force"
    )]
    Put {
        /// The local file to upload.
        local: String,
        /// Where to: a folder (ending in /, or one that exists) or a file path.
        remote: Option<String>,
        /// Replace a file that is already there (its old version is kept, see `versions`).
        #[arg(long, short)]
        force: bool,
    },
    /// Make a folder.
    #[command(after_help = "Examples:\n  craftcloud mkdir Documents/Q3\n  craftcloud mkdir -p Projects/2027/Drafts")]
    Mkdir {
        /// The folder to make.
        path: String,
        /// Make missing parent folders too, and don't mind if it exists.
        #[arg(long, short)]
        parents: bool,
    },
    /// Move or rename a file or folder.
    #[command(after_help = "Examples:\n  craftcloud mv Documents/a.txt Documents/b.txt\n  craftcloud mv Documents/a.txt Archive/")]
    Mv {
        /// What to move.
        from: String,
        /// Where to: a new path, or a folder (ending in /, or one that exists) to move it into.
        to: String,
    },
    /// Copy a file or folder.
    #[command(after_help = "Examples:\n  craftcloud cp Documents/a.txt Documents/a-copy.txt\n  craftcloud cp Photos Backup/")]
    Cp {
        /// What to copy.
        from: String,
        /// Where to: a new path, or a folder (ending in /, or one that exists) to copy it into.
        to: String,
    },
    /// Move files or folders to the trash.
    #[command(after_help = "Example:\n  craftcloud rm Documents/old.txt Documents/Drafts")]
    Rm {
        /// The files and folders.
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// The trash: list, restore, delete for good, empty.
    #[command(subcommand)]
    Trash(TrashCmd),
    /// Star a file or folder.
    #[command(after_help = "Example:\n  craftcloud star Documents/report.pdf")]
    Star {
        /// The file or folder.
        path: String,
    },
    /// Unstar a file or folder.
    #[command(after_help = "Example:\n  craftcloud unstar Documents/report.pdf")]
    Unstar {
        /// The file or folder.
        path: String,
    },
    /// List starred files and folders.
    #[command(after_help = "Examples:\n  craftcloud starred\n  craftcloud starred --app wordcraft --json")]
    Starred {
        /// Also show CraftCloud's own `.craftcloud` folder (kept versions).
        #[arg(long)]
        all: bool,
        /// Only files this Craft app opens (see `craftcloud apps`).
        #[arg(long, value_name = "APP")]
        app: Option<String>,
    },
    /// List recently opened files.
    #[command(after_help = "Examples:\n  craftcloud recent\n  craftcloud recent --app gridcraft --json\n  craftcloud recent --clear")]
    Recent {
        /// Clear the list instead.
        #[arg(long)]
        clear: bool,
        /// Also show CraftCloud's own `.craftcloud` folder (kept versions).
        #[arg(long)]
        all: bool,
        /// Only files this Craft app opens (see `craftcloud apps`).
        #[arg(long, value_name = "APP", conflicts_with = "clear")]
        app: Option<String>,
    },
    /// Search file and folder names.
    #[command(after_help = "Examples:\n  craftcloud search report\n  craftcloud search q3 --type document --in Documents")]
    Search {
        /// Words in the name.
        query: String,
        /// Only this kind: folder, image, video, audio, document, archive, code, font.
        #[arg(long = "type", value_name = "KIND")]
        kind: Option<String>,
        /// Only inside this folder.
        #[arg(long = "in", value_name = "FOLDER")]
        within: Option<String>,
        /// At most this many results.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Also show CraftCloud's own `.craftcloud` folder (kept versions).
        #[arg(long)]
        all: bool,
        /// Only files this Craft app opens (see `craftcloud apps`).
        #[arg(long, value_name = "APP")]
        app: Option<String>,
    },
    /// Share links: create, list, remove.
    #[command(subcommand)]
    Share(ShareCmd),
    /// List a file's earlier versions, or restore one.
    #[command(after_help = "Examples:\n  craftcloud versions Documents/notes.md\n  craftcloud versions Documents/notes.md restore 1")]
    Versions {
        /// The file.
        path: String,
        /// `restore`
        #[arg(value_parser = ["restore"])]
        action: Option<String>,
        /// The version's number in the list (1 = newest).
        n: Option<usize>,
    },
    /// Open the local synced copy of a file with its app.
    #[command(after_help = "Example:\n  craftcloud open Documents/notes.md")]
    Open {
        /// The cloud path of a synced file.
        path: String,
    },
    /// Show storage use.
    #[command(after_help = "Example:\n  craftcloud usage --json")]
    Usage,
    /// The server: what it guarantees the sync (`check`), and its administration (owners and admins).
    #[command(subcommand)]
    Server(ServerCmd),
}

#[derive(Debug, Subcommand)]
pub enum TrashCmd {
    /// List what is in the trash.
    #[command(after_help = "Examples:\n  craftcloud trash ls\n  craftcloud trash ls --json")]
    Ls {
        /// Also show CraftCloud's own `.craftcloud` folder (kept versions).
        #[arg(long)]
        all: bool,
    },
    /// Put an item back where it was.
    #[command(
        after_help = "Examples:\n  craftcloud trash restore Documents/old.txt\n  craftcloud trash restore Documents/old.txt --rename     as \"old (restored).txt\" when old.txt is there\n  craftcloud trash restore Documents/old.txt --as Archive/old.txt"
    )]
    Restore {
        /// Its old path, its name, or its id from `trash ls --json`.
        item: String,
        /// Restore it here instead of where it was.
        #[arg(long = "as", value_name = "PATH", conflicts_with = "rename")]
        to: Option<String>,
        /// When its old place is taken, restore it as "NAME (restored).EXT" next to it.
        #[arg(long)]
        rename: bool,
    },
    /// Delete an item for good.
    #[command(after_help = "Example:\n  craftcloud trash rm Documents/old.txt")]
    Rm {
        /// Its old path, its name, or its id from `trash ls --json`.
        item: String,
    },
    /// Delete everything in the trash for good.
    #[command(after_help = "Example:\n  craftcloud trash empty --yes")]
    Empty {
        /// Don't ask.
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ShareCmd {
    /// Make a link to a file or folder.
    #[command(
        after_help = "Examples:\n  craftcloud share create Documents/report.pdf\n  craftcloud share create Photos --expires 2027-01-31 --password"
    )]
    Create {
        /// The file or folder to share.
        path: String,
        /// When the link stops working: a date (2027-01-31), a time (RFC 3339) or a duration (12h, 7d, 2w).
        #[arg(long, value_name = "WHEN")]
        expires: Option<String>,
        /// Ask for a password for the link (read from the terminal, or the first line of stdin).
        #[arg(long)]
        password: bool,
    },
    /// List links (all, or those to one file or folder), with where each item is now.
    #[command(after_help = "Examples:\n  craftcloud share ls\n  craftcloud share ls Documents/report.pdf --json")]
    Ls {
        /// Only the links to this file or folder.
        path: Option<String>,
    },
    /// Remove a link (by id or URL), or all links to a path.
    #[command(
        after_help = "Examples:\n  craftcloud share rm https://cloud.example.org/s/AbC123\n  craftcloud share rm Documents/report.pdf      every link to it"
    )]
    Rm {
        /// The link's id or URL, or a path (all its links).
        link: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ServerCmd {
    /// What the server is and runs.
    #[command(after_help = "Example:\n  craftcloud server info --json")]
    Info,
    /// What the server guarantees the sync (guarded writes, version tags, a trash, versions) and
    /// the sync mode that allows: two-way, download only or refused. On a CraftCloud server these
    /// are known, not measured.
    #[command(after_help = "Examples:\n  craftcloud server check\n  craftcloud server check --json")]
    Check,
    /// People on the server.
    #[command(subcommand)]
    Users(UsersCmd),
    /// The server's settings (the Craft apps in the browser).
    #[command(subcommand)]
    Settings(SettingsCmd),
}

#[derive(Debug, Subcommand)]
pub enum UsersCmd {
    /// List people.
    #[command(after_help = "Example:\n  craftcloud server users ls --json")]
    Ls,
    /// Add a person (the password is read from the terminal, or the first line of stdin).
    #[command(after_help = "Example:\n  craftcloud server users add jane --email jane@example.org --quota 10GB")]
    Add {
        /// Their username.
        username: String,
        /// Their email address.
        #[arg(long)]
        email: String,
        /// Storage limit, e.g. 10GB (default: the server's).
        #[arg(long, value_name = "SIZE")]
        quota: Option<String>,
        /// Make them an admin (they can manage people and the server).
        #[arg(long)]
        admin: bool,
    },
    /// Set a new password (read from the terminal, or the first line of stdin).
    #[command(after_help = "Example:\n  printf '%s\\n' \"$PW\" | craftcloud server users passwd jane")]
    Passwd {
        /// Their username.
        username: String,
    },
    /// Let a person sign in again.
    #[command(after_help = "Example:\n  craftcloud server users enable jane")]
    Enable {
        /// Their username.
        username: String,
    },
    /// Stop a person from signing in.
    #[command(after_help = "Example:\n  craftcloud server users disable jane")]
    Disable {
        /// Their username.
        username: String,
    },
    /// Set a person's storage limit (`unlimited` for none).
    #[command(after_help = "Examples:\n  craftcloud server users quota jane 10GB\n  craftcloud server users quota jane unlimited")]
    Quota {
        /// Their username.
        username: String,
        /// 10GB, 500MB, 1.5TB, 512MiB, or unlimited.
        size: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SettingsCmd {
    /// Show the settings.
    #[command(after_help = "Example:\n  craftcloud server settings show --json")]
    Show,
    /// Change the settings.
    #[command(after_help = "Examples:\n  craftcloud server settings set --web-apps on\n  craftcloud server settings set --disable-app effectcraft")]
    Set {
        /// Run the Craft apps in the browser (needs the -apps image).
        #[arg(long, value_name = "on|off", value_parser = ["on", "off"])]
        web_apps: Option<String>,
        /// Switch one app off (repeatable).
        #[arg(long, value_name = "APP")]
        disable_app: Vec<String>,
        /// Switch one app back on (repeatable).
        #[arg(long, value_name = "APP")]
        enable_app: Vec<String>,
    },
}

/// Runs a command when the command line names one; `None` means: start the desktop client.
pub fn run_if_command() -> Option<ExitCode> {
    // `craftcloud craftcloud://open?path=…`: how the system starts us for a link (docs/CLI.md).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(url) = link_arg(&args) {
        let json = args.iter().any(|a| a == "--json");
        let result = account::link(&Ctx::new(json, None), url);
        return Some(match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                // Started from a browser there is no terminal to read the message in.
                if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
                    craftcloud_native::open::tell_user(&e.message);
                }
                e.exit(json)
            }
        });
    }
    let cli = Cli::parse();
    let command = cli.command?;
    quiet_on_closed_pipe();
    let json = cli.global.json;
    let mut ctx = Ctx::new(json, cli.global.server);
    Some(match run(&mut ctx, command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => e.exit(json),
    })
}

/// The link, when the first argument that isn't a global option (`--json`, `--server URL`,
/// `--server=URL`) is one: `craftcloud --json 'craftcloud://…'` is a link too (T-026). A link
/// opens the synced copy in the desktop client's folder, so `--server` changes nothing for it.
fn link_arg(args: &[String]) -> Option<&str> {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => {}
            "--server" => {
                rest.next()?;
            }
            a if a.starts_with("--server=") => {}
            a => return is_link(a).then_some(a),
        }
    }
    None
}

/// Whether an argument is a `craftcloud:` link rather than a command.
fn is_link(arg: &str) -> bool {
    arg.get(..craftcloud_core::deeplink::SCHEME.len() + 1).is_some_and(|p| p.eq_ignore_ascii_case(&format!("{}:", craftcloud_core::deeplink::SCHEME)))
}

/// `craftcloud ls | head` closes stdout early; `println!` then panics ("failed printing to stdout:
/// Broken pipe"). Like other command-line tools, end quietly with success instead.
fn quiet_on_closed_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied()).unwrap_or("");
        if is_closed_pipe(message) {
            std::process::exit(0);
        }
        default(info);
    }));
}

fn is_closed_pipe(panic_message: &str) -> bool {
    panic_message.contains("failed printing to stdout") && panic_message.contains("Broken pipe")
}

fn run(ctx: &mut Ctx, command: Cmd) -> Result<()> {
    // A settings file naming another kind of server is refused by a build without the opt-in
    // (docs/WEBDAV-STORAGE.md § 16), before anything is read from or written to a server or the
    // sync folder. Signing out (which forgets it) and the built-in app list still work.
    if !matches!(command, Cmd::Logout | Cmd::Apps) {
        ctx.allowed()?;
    }
    match command {
        Cmd::Login { server, no_browser, kind, user } => account::login(ctx, server.as_deref(), no_browser, kind.as_deref(), user.as_deref()),
        Cmd::Logout => account::logout(ctx),
        Cmd::Status => account::status(ctx),
        Cmd::Sync { no_wait, once, confirm_deletes, discard_deletes } => {
            let deletes = if confirm_deletes {
                account::Deletes::Confirm
            } else if discard_deletes {
                account::Deletes::Discard
            } else {
                account::Deletes::Hold
            };
            account::sync(ctx, !no_wait, once, deletes)
        }
        Cmd::Daemon => headless::daemon(ctx),
        Cmd::Pause => account::pause(ctx, true),
        Cmd::Resume => account::pause(ctx, false),
        Cmd::Usage => account::usage(ctx),
        Cmd::Open { path } => account::open(ctx, &path),
        Cmd::Ls { path, all, app } => files::ls(ctx, &path, all, app.as_deref()),
        Cmd::Apps => craft::apps(ctx),
        Cmd::Files { app, within } => craft::files(ctx, app.as_deref(), within.as_deref()),
        Cmd::Save { local, to, app, force, if_match, wait } => {
            craft::save(ctx, &craft::SaveArgs { local: &local, to: to.as_deref(), app: app.as_deref(), force, wait, if_match: if_match.as_deref() })
        }
        Cmd::Get { remote, local, force } => files::get(ctx, &remote, local.as_deref(), force),
        Cmd::Put { local, remote, force } => files::put(ctx, &local, remote.as_deref(), force),
        Cmd::Mkdir { path, parents } => files::mkdir(ctx, &path, parents),
        Cmd::Mv { from, to } => files::move_or_copy(ctx, &from, &to, false),
        Cmd::Cp { from, to } => files::move_or_copy(ctx, &from, &to, true),
        Cmd::Rm { paths } => files::rm(ctx, &paths),
        Cmd::Trash(t) => files::trash(ctx, t),
        Cmd::Star { path } => files::star(ctx, &path, true),
        Cmd::Unstar { path } => files::star(ctx, &path, false),
        Cmd::Starred { all, app } => files::starred(ctx, all, app.as_deref()),
        Cmd::Recent { clear, all, app } => files::recent(ctx, clear, all, app.as_deref()),
        Cmd::Search { query, kind, within, limit, all, app } => files::search(
            ctx,
            &files::SearchArgs { query: &query, kind: kind.as_deref(), within: within.as_deref(), limit, all, app: app.as_deref() },
        ),
        Cmd::Share(s) => sharing::share(ctx, s),
        Cmd::Versions { path, action, n } => sharing::versions(ctx, &path, action.is_some(), n),
        Cmd::Server(s) => admin::server(ctx, s),
    }
}

/// Set to `1` by `craftcloud.com`, the Windows terminal launcher (`apps/craftcloud-cli`), when it
/// relays a console to our stdin: a pipe, but a person types into it.
const STDIN_CONSOLE: &str = "CRAFTCLOUD_STDIN_CONSOLE";

/// Whether a person types our input, so a question may be asked: a terminal, or a Windows console
/// that `craftcloud.com` relays.
pub fn stdin_is_terminal() -> bool {
    typed_input(std::io::IsTerminal::is_terminal(&std::io::stdin()), std::env::var_os(STDIN_CONSOLE).as_deref())
}

fn typed_input(is_terminal: bool, relayed_console: Option<&std::ffi::OsStr>) -> bool {
    is_terminal || relayed_console.is_some_and(|v| v == "1")
}

/// Reads a secret: from the terminal without echo, or the first line of stdin when it isn't one.
pub fn read_secret(prompt: &str) -> Result<String> {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let tty = stdin_is_terminal();
    if tty {
        eprint!("{prompt}: ");
        echo(false);
    }
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if tty {
        echo(true);
        eprintln!();
    }
    read.map_err(|e| CliError::error(format!("can't read the password: {e}")))?;
    let secret = line.trim_end_matches(['\n', '\r']).to_owned();
    if secret.is_empty() {
        return Err(CliError::usage("no password given"));
    }
    Ok(secret)
}

/// Terminal echo on or off (Unix, through `stty`; elsewhere the typing shows).
fn echo(on: bool) {
    if cfg!(unix) {
        let _ = std::process::Command::new("stty").arg(if on { "echo" } else { "-echo" }).stdin(std::process::Stdio::inherit()).status();
    }
}

#[cfg(test)]
mod tests;
