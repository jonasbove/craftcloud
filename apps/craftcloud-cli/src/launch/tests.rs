//! The launcher with a stand-in child (`/bin/sh`): arguments arrive unchanged, the exit code comes
//! back, relayed streams pass through, and Ctrl+C stops the child.

use std::sync::mpsc;

use super::{Input, Output, Wiring, client_next_to, run};

fn inherit() -> Wiring {
    Wiring { stdin: Input::Inherit, stdout: Output::Inherit, stderr: Output::Inherit }
}

/// No sender: Ctrl+C never comes.
fn never() -> mpsc::Receiver<()> {
    mpsc::channel().1
}

#[test]
fn a_missing_program_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(run(&dir.path().join("nothing-here"), &[], inherit(), &never()).is_err());
}

#[test]
fn finds_the_client_next_to_itself() {
    let dir = tempfile::tempdir().unwrap();
    let client = dir.path().join(format!("craftcloud{}", std::env::consts::EXE_SUFFIX));
    let me = dir.path().join("craftcloud.com");
    assert!(client_next_to(&me).unwrap_err().contains("can't find"));
    std::fs::write(&client, b"").unwrap();
    assert_eq!(client_next_to(&me).unwrap(), client);
    // Under the client's own name it would start itself.
    assert!(client_next_to(&client).unwrap_err().contains("must not be called"));
}

#[cfg(unix)]
mod sh {
    use std::ffi::OsString;
    use std::io::{self, Cursor, Write};
    use std::path::Path;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::super::{Input, Outcome, Output, Wiring, run};
    use super::{inherit, never};

    /// A writer the test can read back.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    const SH: &str = "/bin/sh";

    #[test]
    fn passes_every_argument_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("args");
        let mut list = args(&["-c", r#"printf '%s\n' "$@" > "$0""#]);
        list.push(file.clone().into_os_string());
        list.extend(args(&["status", "two words", "", "--json", "ünïcödé", "a\"quote"]));
        let outcome = run(Path::new(SH), &list, inherit(), &never()).unwrap();
        assert_eq!(outcome, Outcome::Exited(0));
        assert_eq!(std::fs::read_to_string(file).unwrap(), "status\ntwo words\n\n--json\nünïcödé\na\"quote\n");
    }

    #[test]
    fn exits_with_the_childs_code() {
        for code in [0, 1, 2, 7, 255] {
            let outcome = run(Path::new(SH), &args(&["-c", &format!("exit {code}")]), inherit(), &never()).unwrap();
            assert_eq!(outcome, Outcome::Exited(code));
        }
    }

    #[test]
    fn relays_output_and_input() {
        let (out, err) = (Shared::default(), Shared::default());
        let wiring = Wiring {
            stdin: Input::Relay(Box::new(Cursor::new(b"secret\n".to_vec()))),
            stdout: Output::Relay(Box::new(out.clone())),
            stderr: Output::Relay(Box::new(err.clone())),
        };
        let script = r#"printf 'Password: ' >&2; read line; printf 'got %s (%s)\n' "$line" "$CRAFTCLOUD_STDIN_CONSOLE"; exit 3"#;
        let outcome = run(Path::new(SH), &args(&["-c", script]), wiring, &never()).unwrap();
        assert_eq!(outcome, Outcome::Exited(3));
        // The child is told that a person types into that pipe.
        assert_eq!(out.text(), "got secret (1)\n");
        assert_eq!(err.text(), "Password: ");
    }

    #[test]
    fn ctrl_c_stops_the_child() {
        let (interrupt, interrupted) = mpsc::channel();
        interrupt.send(()).unwrap();
        let started = Instant::now();
        let outcome = run(Path::new(SH), &args(&["-c", "sleep 30"]), inherit(), &interrupted).unwrap();
        assert_eq!(outcome, Outcome::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }
}
