//! Running the desktop client for a terminal: find it, start it with the same arguments, pass
//! input and output through, and hand back its exit code.
//!
//! Why the output is copied rather than handed on: a console handle (Windows 8 and later) works
//! only in a process that has a console attached, and a GUI-subsystem program has none. So where
//! one of this program's streams is a console, the child gets a pipe instead and a thread copies
//! between the two. Files and pipes (`craftcloud ls > out.txt`, `craftcloud ls | more`) are handed
//! on as they are, as they always worked.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

/// The desktop client's name, without `.exe`.
const CLIENT: &str = "craftcloud";

/// Set to `1` for the child when its input is relayed from a console: its stdin is a pipe then,
/// but a person is typing, so it may still ask (a password, "Delete …? [y/N]"). The client reads
/// it in `apps/craftcloud/src/cli/mod.rs` (`stdin_is_terminal`).
pub const STDIN_CONSOLE: &str = "CRAFTCLOUD_STDIN_CONSOLE";

/// The exit code after Ctrl+C: what Windows gives a console program it stops
/// (`STATUS_CONTROL_C_EXIT`), and the shells' 128 + SIGINT elsewhere.
pub const INTERRUPTED: i32 = if cfg!(windows) { 0xC000_013A_u32 as i32 } else { 130 };

/// How often the wait looks for Ctrl+C.
const POLL: Duration = Duration::from_millis(50);

/// How long, after the child exits, its output may take to arrive. Something it started that kept
/// the pipe open would otherwise hold the terminal.
const DRAIN: Duration = Duration::from_secs(2);

/// Where a stream of the child comes from.
pub enum Input {
    /// The child gets this program's own handle.
    Inherit,
    /// The child reads a pipe that a thread fills from this reader.
    Relay(Box<dyn Read + Send>),
}

/// Where a stream of the child goes.
pub enum Output {
    /// The child gets this program's own handle.
    Inherit,
    /// The child writes a pipe that a thread copies into this writer.
    Relay(Box<dyn Write + Send>),
}

pub struct Wiring {
    pub stdin: Input,
    pub stdout: Output,
    pub stderr: Output,
}

impl Wiring {
    /// This program's streams: copied through where they are a Windows console, else handed on.
    pub fn for_this_process() -> Wiring {
        let console = |is_terminal: bool| cfg!(windows) && is_terminal;
        Wiring {
            stdin: if console(io::stdin().is_terminal()) { Input::Relay(Box::new(io::stdin())) } else { Input::Inherit },
            stdout: if console(io::stdout().is_terminal()) { Output::Relay(Box::new(io::stdout())) } else { Output::Inherit },
            stderr: if console(io::stderr().is_terminal()) { Output::Relay(Box::new(io::stderr())) } else { Output::Inherit },
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The child ended with this exit code.
    Exited(i32),
    /// Ctrl+C: the child was stopped.
    Interrupted,
}

/// `craftcloud.exe` in the folder `me` is in, where the installer and the portable zip put both.
pub fn client_next_to(me: &Path) -> Result<PathBuf, String> {
    let name = format!("{CLIENT}{}", std::env::consts::EXE_SUFFIX);
    // Renamed to the client's own name, this program would start itself over and over.
    if me.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name)) {
        return Err(format!("this program must not be called {name}: it runs {name} next to it"));
    }
    let client = me.parent().map(|dir| dir.join(&name)).ok_or_else(|| format!("can't find {name}: no folder"))?;
    if !client.is_file() {
        return Err(format!("can't find {}: it belongs next to this program", client.display()));
    }
    Ok(client)
}

/// Starts the window and returns at once, attached to nothing here.
pub fn start_window(program: &Path) -> io::Result<()> {
    Command::new(program).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(drop)
}

/// Runs `program` with `args` and waits for it. A message on `interrupt` stops it (Ctrl+C); when
/// `interrupt` has no sender left, the wait just waits.
pub fn run(program: &Path, args: &[OsString], wiring: Wiring, interrupt: &Receiver<()>) -> io::Result<Outcome> {
    let mut command = Command::new(program);
    command.args(args).stdout(stdio(&wiring.stdout)).stderr(stdio(&wiring.stderr));
    match wiring.stdin {
        Input::Inherit => command.stdin(Stdio::inherit()).env_remove(STDIN_CONSOLE),
        Input::Relay(_) => command.stdin(Stdio::piped()).env(STDIN_CONSOLE, "1"),
    };
    let mut child = command.spawn()?;

    if let (Input::Relay(mut from), Some(mut to)) = (wiring.stdin, child.stdin.take()) {
        // Never joined: it may sit in a read of the terminal after the child is gone, and ends
        // with this process. Closing the pipe at the end of `from` tells the child.
        thread::spawn(move || {
            let _ = io::copy(&mut from, &mut to);
        });
    }
    let (done, finished) = mpsc::channel();
    let mut pumps = 0;
    let out = child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>);
    let err = child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>);
    for (sink, pipe) in [(wiring.stdout, out), (wiring.stderr, err)] {
        if let (Output::Relay(mut to), Some(mut from)) = (sink, pipe) {
            pumps += 1;
            let done = done.clone();
            thread::spawn(move || {
                pump(&mut from, &mut to);
                let _ = done.send(());
            });
        }
    }

    let Some(code) = wait(&mut child, interrupt)? else {
        return Ok(Outcome::Interrupted);
    };
    for _ in 0..pumps {
        if finished.recv_timeout(DRAIN).is_err() {
            break;
        }
    }
    Ok(Outcome::Exited(code))
}

fn stdio(output: &Output) -> Stdio {
    match output {
        Output::Inherit => Stdio::inherit(),
        Output::Relay(_) => Stdio::piped(),
    }
}

/// The child's exit code, or `None` when `interrupt` stopped it first.
fn wait(child: &mut Child, interrupt: &Receiver<()>) -> io::Result<Option<i32>> {
    loop {
        if let Some(status) = child.try_wait()? {
            // No code only when a Unix signal ended it.
            return Ok(Some(status.code().unwrap_or(1)));
        }
        match interrupt.recv_timeout(POLL) {
            Ok(()) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return child.wait().map(|s| Some(s.code().unwrap_or(1))),
        }
    }
}

/// Copies as the bytes arrive, flushing each piece, so a prompt without a line end shows at once.
fn pump(from: &mut dyn Read, to: &mut dyn Write) {
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if buf.get(..n).is_none_or(|bytes| to.write_all(bytes).and_then(|()| to.flush()).is_err()) {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests;
