//! `craftcloud.com`: the `craftcloud` command line in a Windows terminal.
//!
//! `craftcloud.exe` is a GUI-subsystem program (no console window opens with the client), so
//! Windows gives it no console: typed in Command Prompt or PowerShell, its output went nowhere
//! (#4). This small console program sits next to it as `craftcloud.com`. Both shells try `.com`
//! before `.exe` (PATHEXT), so `craftcloud status` runs this, which runs `craftcloud.exe` with the
//! same arguments, passes its output and input through, and exits with its exit code. Shortcuts,
//! the start at login and `craftcloud://` links name `craftcloud.exe` and never come here.
//!
//! On Linux and macOS the client is a console program already; this builds there only so the
//! workspace's checks and tests cover it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod launch;

use std::ffi::OsString;
use std::io::Write as _;
use std::sync::mpsc;

use crate::launch::{Outcome, Wiring};

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let code = match run(&args) {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "craftcloud: {message}");
            1
        }
    };
    std::process::exit(code);
}

fn run(args: &[OsString]) -> Result<i32, String> {
    let program = std::env::current_exe().map_err(|e| format!("can't tell where this program is: {e}")).and_then(|me| launch::client_next_to(&me))?;
    if args.is_empty() {
        // `craftcloud` alone opens the window: start it and give the terminal back, as the
        // Start menu would. (Win+R "craftcloud" may come here too, PATH before App Paths.)
        launch::start_window(&program).map_err(|e| format!("can't start {}: {e}", program.display()))?;
        return Ok(0);
    }
    let (interrupt, interrupted) = mpsc::channel::<()>();
    #[cfg(windows)]
    {
        // Ctrl+C and Ctrl+Break go to the processes attached to the console: this one, not the
        // GUI child. Stop the child for them; without the handler it would keep running.
        if let Err(e) = ctrlc::set_handler(move || {
            let _ = interrupt.send(());
        }) {
            let _ = writeln!(std::io::stderr(), "craftcloud: Ctrl+C won't stop the command: {e}");
        }
    }
    // Elsewhere Ctrl+C reaches the whole foreground process group, the child included.
    #[cfg(not(windows))]
    drop(interrupt);
    match launch::run(&program, args, Wiring::for_this_process(), &interrupted) {
        Ok(Outcome::Exited(code)) => Ok(code),
        Ok(Outcome::Interrupted) => Ok(launch::INTERRUPTED),
        Err(e) => Err(format!("can't run {}: {e}", program.display())),
    }
}
