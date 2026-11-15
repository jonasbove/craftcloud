//! One scratch client: the `craftcloud` command line with a settings folder, a home and a sync
//! folder of its own, its tokens in a file there (`CRAFTCLOUD_KEYCHAIN=off`), never the keychain.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::api::Api;

pub struct Client {
    /// Its device name, used in conflict copies: "Laptop" or "Studio".
    pub name: &'static str,
    pub root: PathBuf,
    /// The sync folder.
    pub folder: PathBuf,
    config: PathBuf,
    home: PathBuf,
    bin: PathBuf,
    pub signed_in: bool,
}

impl Client {
    pub fn new(name: &'static str, root: PathBuf, bin: &Path) -> Result<Self, String> {
        let folder = root.join("folder");
        let config = root.join("config");
        let home = root.join("home");
        for dir in [&folder, &config, &home] {
            fs::create_dir_all(dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
        }
        Ok(Self { name, root, folder, config, home, bin: bin.to_path_buf(), signed_in: false })
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(args)
            .env("CRAFTCLOUD_CONFIG_DIR", &self.config)
            .env("CRAFTCLOUD_KEYCHAIN", "off")
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("XDG_STATE_HOME", self.home.join(".local/state"))
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            .env_remove("CRAFTCLOUD_SERVER")
            .env_remove("CRAFTCLOUD_TEST_PASSWORD")
            .current_dir(&self.root)
            .stdin(Stdio::null());
        c
    }

    /// Runs a command with its stdout in `out` and its stderr next to it (`.err`, removed when
    /// empty). Its exit code; an error when it couldn't start or ran past `timeout` (then it is
    /// stopped).
    pub fn run(&self, args: &[&str], out: &Path, timeout: Duration) -> Result<i32, String> {
        let err = out.with_extension("err");
        let stdout = File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let stderr = File::create(&err).map_err(|e| format!("{}: {e}", err.display()))?;
        let mut child =
            self.command(args).stdout(stdout).stderr(stderr).spawn().map_err(|e| format!("couldn't start {}: {e}", self.bin.display()))?;
        let code = wait_for(&mut child, timeout).map_err(|e| format!("`craftcloud {}` on {}: {e}", args.join(" "), self.name));
        if fs::metadata(&err).is_ok_and(|m| m.len() == 0) {
            let _ = fs::remove_file(&err);
        }
        code
    }

    /// The settings the run needs, before the sign-in fills in the server: the sync folder, this
    /// device's name, no files at the top of the cloud, nothing excluded yet.
    pub fn write_config(&self) -> Result<(), String> {
        let text = format!(
            "schema_version = 1\nlocal_folder = {}\nexcluded = []\nfolders = []\nsync_root_files = false\ndevice_name = {}\n",
            toml_str(&self.folder.to_string_lossy()),
            toml_str(self.name)
        );
        let path = self.config.join("config.toml");
        fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Switches off every top-level cloud folder in `names` (the sync then takes only the run's).
    pub fn exclude(&self, names: &[String]) -> Result<(), String> {
        let path = self.config.join("config.toml");
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let list = format!("excluded = [{}]", names.iter().map(|n| toml_str(n)).collect::<Vec<_>>().join(", "));
        if !text.lines().any(|l| l.trim_start().starts_with("excluded =")) {
            return Err(format!("{}: no `excluded` line to change (has the settings format changed?)", path.display()));
        }
        let lines: Vec<String> = text.lines().map(|l| if l.trim_start().starts_with("excluded =") { list.clone() } else { l.to_owned() }).collect();
        fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Signs in through the device flow (`login --no-browser --json`), approving the code through
    /// the API as the person would on the `/device` page. The output goes to `log`.
    pub fn login(&mut self, server: &str, api: &Api, log: &Path) -> Result<(), String> {
        let err = File::create(log.with_extension("err")).map_err(|e| format!("{}: {e}", log.display()))?;
        let mut child = self
            .command(&["login", server, "--no-browser", "--json"])
            .stdout(Stdio::piped())
            .stderr(err)
            .spawn()
            .map_err(|e| format!("couldn't start {}: {e}", self.bin.display()))?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no stdout from `craftcloud login`".to_owned());
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut seen = Vec::new();
        let mut next = |wait: u64| -> Result<Value, String> {
            let line = rx.recv_timeout(Duration::from_secs(wait)).map_err(|_| "`craftcloud login` said nothing more".to_owned())?;
            seen.push(line.clone());
            serde_json::from_str(&line).map_err(|e| format!("`craftcloud login` printed something that isn't JSON: {e}"))
        };
        let result = (|| {
            let start = next(60)?;
            let code = start["user_code"].as_str().ok_or("`craftcloud login` printed no user_code")?;
            api.approve(code)?;
            let done = next(120)?;
            if done["signed_in"] != true {
                return Err(format!("`craftcloud login` ended with {done}"));
            }
            match wait_for(&mut child, Duration::from_secs(30))? {
                0 => Ok(()),
                code => Err(format!("`craftcloud login` exited with {code}")),
            }
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::write(log, seen.join("\n") + "\n");
        self.signed_in = result.is_ok();
        result.map_err(|e| format!("signing in {}: {e}", self.name))
    }
}

/// Waits for a child, stopping it after `timeout`.
fn wait_for(child: &mut Child, timeout: Duration) -> Result<i32, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("stopped after {} s", timeout.as_secs()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    }
}

/// A TOML basic string.
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::toml_str;

    #[test]
    fn toml_strings_are_escaped() {
        assert_eq!(toml_str(r#"Wéb "ny" \ mappe"#), r#""Wéb \"ny\" \\ mappe""#);
        assert_eq!(toml_str("a\tb"), r#""a\u0009b""#);
    }
}
