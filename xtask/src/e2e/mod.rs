//! `cargo xtask e2e-sync`: the two-client sync smoke test against a real server (docs/TESTING.md).
//!
//! Two scratch clients, "Laptop" and "Studio", each the `craftcloud` command line with its own
//! settings folder and sync folder, signed in as separate devices of the test account. They sync a
//! fresh top-level folder of their own (every other folder switched off) one `sync --once` pass at
//! a time, and the cases in [`cases`] check what arrived on the other side. Every pass's `--json`
//! is kept under the run directory; the result is a PASS/FAIL table and the exit code.

mod api;
mod cases;
mod client;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::{Api, Session};
use client::Client;

const USAGE: &str = "\
usage: cargo xtask e2e-sync [--bin PATH | --bin-laptop PATH --bin-studio PATH] [--keep] [--settle SECS] [--dir PATH] [--allow-other-devices]

Two scratch clients sync a fresh folder of the test account and check every change on the other side.
Needs CRAFTCLOUD_TEST_SERVER, CRAFTCLOUD_TEST_USER and CRAFTCLOUD_TEST_PASSWORD (a test account: the
run signs two clients in to it). See docs/TESTING.md.

  --bin PATH               the craftcloud binary both clients run (default: build this tree's, debug)
  --bin-laptop PATH        the Laptop's binary, instead of --bin (a mixed pair: an older release on one side)
  --bin-studio PATH        the Studio's binary, instead of --bin
  --keep                   keep the cloud folder and the clients' folders afterwards
  --settle SECS            the wait before every pass, more than 2 (files are held 2 s after a change); default 3
  --dir PATH               where the run directory goes (default: <target>/e2e-sync)
  --allow-other-devices    go on, with a warning, when other clients are signed in (admin accounts)
";

/// How long one `sync --once` pass may take before it counts as hung (and is stopped).
const PASS_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long any other command may take.
const CMD_TIMEOUT: Duration = Duration::from_secs(120);

struct Options {
    bin: Option<PathBuf>,
    bin_laptop: Option<PathBuf>,
    bin_studio: Option<PathBuf>,
    keep: bool,
    settle: Duration,
    dir: Option<PathBuf>,
    allow_other_devices: bool,
}

/// Whether a trashed item's path (`<owner>/<path>`) is the run's cloud folder `cloud`, inside it, or
/// among CraftCloud's kept versions of its files: what the run may clean up, nothing else.
fn of_run(cloud: &str, trashed: &str) -> bool {
    let rel = trashed.split_once('/').map_or(trashed, |(_, rest)| rest);
    [cloud.to_owned(), format!(".craftcloud/versions/{cloud}")].iter().any(|top| rel == top || rel.starts_with(&format!("{top}/")))
}

fn options(args: &[String]) -> Result<Option<Options>, String> {
    let mut o =
        Options { bin: None, bin_laptop: None, bin_studio: None, keep: false, settle: Duration::from_secs(3), dir: None, allow_other_devices: false };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value\n\n{USAGE}"));
        match arg.as_str() {
            "--bin" => o.bin = Some(PathBuf::from(value()?)),
            "--bin-laptop" => o.bin_laptop = Some(PathBuf::from(value()?)),
            "--bin-studio" => o.bin_studio = Some(PathBuf::from(value()?)),
            "--dir" => o.dir = Some(PathBuf::from(value()?)),
            "--keep" => o.keep = true,
            "--allow-other-devices" => o.allow_other_devices = true,
            "--settle" => {
                let secs: f64 = value()?.parse().map_err(|_| "--settle takes a number of seconds".to_owned())?;
                // T-060: a file is held for 2 s after a change, so a pass sooner than that skips it.
                if !(secs > 2.0 && secs <= 60.0) {
                    return Err("--settle must be more than 2 s (files are held 2 s after a change) and at most 60".to_owned());
                }
                o.settle = Duration::from_secs_f64(secs);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
        }
    }
    Ok(Some(o))
}

impl Options {
    /// The program each client runs, `(Laptop, Studio)`: its own option, else `--bin`, else
    /// `None` (this tree's build).
    fn bins(&self) -> (Option<&PathBuf>, Option<&PathBuf>) {
        (self.bin_laptop.as_ref().or(self.bin.as_ref()), self.bin_studio.as_ref().or(self.bin.as_ref()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail,
    Warn,
}

struct Row {
    case: String,
    verdict: Verdict,
    detail: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Laptop,
    Studio,
}

impl Who {
    fn other(self) -> Self {
        match self {
            Self::Laptop => Self::Studio,
            Self::Studio => Self::Laptop,
        }
    }
}

/// One run: the two clients, the cloud folder they share and what happened so far.
pub struct Run {
    server: String,
    api: Api,
    laptop: Client,
    studio: Client,
    /// The run's top-level cloud folder, `e2e-sync-<seconds>-<pid>`.
    cloud: String,
    dir: PathBuf,
    settle: Duration,
    passes: Vec<(String, i32)>,
    calls: usize,
    rows: Vec<Row>,
}

pub fn cmd(args: &[String], root: &Path) -> Result<(), String> {
    let Some(opts) = options(args)? else { return Ok(()) };
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty()).ok_or_else(|| format!("{key} is not set\n\n{USAGE}"));
    let (server, user, password) = (env("CRAFTCLOUD_TEST_SERVER")?, env("CRAFTCLOUD_TEST_USER")?, env("CRAFTCLOUD_TEST_PASSWORD")?);
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let (laptop_bin, studio_bin) = opts.bins();
    let built = target.join("debug").join(format!("craftcloud{}", std::env::consts::EXE_SUFFIX));
    if laptop_bin.is_none() || studio_bin.is_none() {
        super::cargo(&["build", "-p", "craftcloud", "--locked"])?;
    }
    // Absolute: each client runs in a folder of its own.
    let absolute = |bin: &Path| fs::canonicalize(bin).ok().filter(|b| b.is_file()).ok_or_else(|| format!("{}: no such program", bin.display()));
    let laptop_bin = absolute(laptop_bin.unwrap_or(&built))?;
    let studio_bin = absolute(studio_bin.unwrap_or(&built))?;
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let cloud = format!("e2e-sync-{secs}-{}", std::process::id());
    let dir = opts.dir.clone().unwrap_or_else(|| target.join("e2e-sync")).join(&cloud);
    for sub in ["passes", "log"] {
        fs::create_dir_all(dir.join(sub)).map_err(|e| format!("couldn't make {}: {e}", dir.join(sub).display()))?;
    }
    let dir = fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (laptop_version, studio_version) = (program_version(&laptop_bin), program_version(&studio_bin));
    eprintln!(
        "e2e-sync: Laptop {} ({laptop_version}), Studio {} ({studio_version}) against {server}, run directory {}",
        laptop_bin.display(),
        studio_bin.display(),
        dir.display()
    );

    let local = other_local_clients();
    let (laptop, studio) = (Client::new("Laptop", dir.join("laptop"), &laptop_bin)?, Client::new("Studio", dir.join("studio"), &studio_bin)?);
    let api = Api::sign_in(&server, &user, &password)?;
    let before = match api.sessions() {
        Ok(before) => before,
        Err(e) => {
            api.sign_out();
            return Err(e);
        }
    };
    let mut run = Run { server, api, laptop, studio, cloud, dir, settle: opts.settle, passes: Vec::new(), calls: 0, rows: Vec::new() };
    // What the run tested, so a mixed pair's report reads on its own.
    // A build between releases says the last release's number, so the paths say which is which.
    let pair = if laptop_bin == studio_bin {
        format!("the same program on both: {}", laptop_bin.display())
    } else {
        format!("a mixed pair: Laptop {}, Studio {}", laptop_bin.display(), studio_bin.display())
    };
    run.row(&format!("clients: Laptop runs {laptop_version}, Studio runs {studio_version}"), Verdict::Pass, &pair);
    if run.preflight(before.as_deref(), &local, opts.allow_other_devices) {
        match run.setup() {
            Ok(()) => {
                cases::all(&mut run);
                run.after(before.as_deref());
            }
            Err(e) => run.row("setup: two clients signed in, syncing a fresh folder", Verdict::Fail, &e),
        }
    }
    run.cleanup(opts.keep, before.as_deref(), !opts.allow_other_devices);
    run.report()
}

impl Run {
    fn client(&self, who: Who) -> &Client {
        match who {
            Who::Laptop => &self.laptop,
            Who::Studio => &self.studio,
        }
    }

    fn row(&mut self, case: &str, verdict: Verdict, detail: &str) {
        let mark = match verdict {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Warn => "WARN",
        };
        eprintln!("e2e-sync: {mark}  {case}{}", if detail.is_empty() { String::new() } else { format!("  ({detail})") });
        self.rows.push(Row { case: case.to_owned(), verdict, detail: detail.to_owned() });
    }

    /// No other client may sync the account during the run: an old client once synced the test
    /// folder too and made a lost file out of its own stale view. With an admin account the server
    /// lists the account's sessions, and another client's stops the run (unless allowed). Without
    /// one the server can't say, so the run warns loudly and relies on the end-state check; other
    /// `craftcloud` processes on this computer warn too, as they may use another account.
    fn preflight(&mut self, before: Option<&[Session]>, local: &[String], allow: bool) -> bool {
        let case = "no other client syncs the account";
        let here = if local.is_empty() { "none on this computer".to_owned() } else { format!("craftcloud processes here: {}", local.join(", ")) };
        if !local.is_empty() {
            eprintln!("\n!!! e2e-sync: other craftcloud processes run on this computer: {}", local.join(", "));
            eprintln!("!!! if one syncs the test account, it takes part in the run and its results mean nothing\n");
        }
        let Some(sessions) = before else {
            eprintln!("\n!!! e2e-sync: the test account isn't an admin, so the server can't list its signed-in clients.");
            eprintln!("!!! Make sure no other client syncs it: one that does takes part in the run and fakes failures.\n");
            self.row(case, Verdict::Warn, &format!("not checkable on the server without admin rights; {here}"));
            return true;
        };
        let others: Vec<String> = sessions.iter().filter(|s| s.origin == "device").map(describe).collect();
        match (others.is_empty(), allow) {
            (true, _) if local.is_empty() => {
                self.row(case, Verdict::Pass, "no client signed in to the account (its sessions); none on this computer")
            }
            (true, _) => self.row(case, Verdict::Warn, &format!("no client signed in to the account; {here}")),
            (false, true) => {
                eprintln!("\n!!! e2e-sync: other clients are signed in to the test account: {}\n", others.join(", "));
                self.row(case, Verdict::Warn, &format!("other clients signed in (allowed): {}", others.join(", ")));
            }
            (false, false) => {
                let why = format!(
                    "other clients are signed in to the account: {}. Sign them out (or use another account), or pass --allow-other-devices",
                    others.join(", ")
                );
                self.row(case, Verdict::Fail, &why);
                return false;
            }
        }
        true
    }

    /// Signs both clients in, makes the run's cloud folder and switches every other folder off.
    fn setup(&mut self) -> Result<(), String> {
        let server = self.server.clone();
        for who in [Who::Laptop, Who::Studio] {
            self.client(who).write_config()?;
            let log = self.dir.join("log").join(format!("login-{}.json", self.client(who).name.to_lowercase()));
            match who {
                Who::Laptop => self.laptop.login(&server, &self.api, &log)?,
                Who::Studio => self.studio.login(&server, &self.api, &log)?,
            }
        }
        let top = self.cli_json(Who::Laptop, &["ls"])?;
        let others: Vec<String> = top
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|i| i["type"] == "folder")
            .filter_map(|i| i["name"].as_str().map(str::to_owned))
            .collect();
        let cloud = self.cloud.clone();
        self.cli_json(Who::Laptop, &["mkdir", &cloud])?;
        self.laptop.exclude(&others)?;
        self.studio.exclude(&others)?;
        self.sync(Who::Laptop);
        self.sync(Who::Studio);
        for who in [Who::Laptop, Who::Studio] {
            if !self.path(who, "").is_dir() {
                return Err(format!("{} didn't get the run's folder {cloud}", self.client(who).name));
            }
        }
        Ok(())
    }

    /// The rows only the whole run can say: every pass ended up to date, and no other client was
    /// active meanwhile (when the account can list its sessions).
    fn after(&mut self, before: Option<&[Session]>) {
        let bad: Vec<String> = self.passes.iter().filter(|(_, code)| *code != 0).map(|(name, code)| format!("{name}: exit {code}")).collect();
        let (verdict, detail) =
            if bad.is_empty() { (Verdict::Pass, format!("{} passes", self.passes.len())) } else { (Verdict::Fail, bad.join(", ")) };
        self.row("every pass ended up to date (exit 0)", verdict, &detail);
        let case = "no other client active during the run";
        let Some(before) = before else {
            self.row(case, Verdict::Warn, "not checkable without admin rights; the end-state comparison above is the check");
            return;
        };
        match self.api.sessions() {
            Ok(Some(now)) => {
                let devices: Vec<&Session> = now.iter().filter(|s| s.origin == "device").collect();
                let new = devices.iter().filter(|s| !before.iter().any(|b| b.id == s.id)).count();
                // An older client that was used during the run: its last sign of life moved.
                let active: Vec<String> =
                    devices.iter().filter(|s| before.iter().any(|b| b.id == s.id && b.last_seen_at != s.last_seen_at)).map(|s| describe(s)).collect();
                if new > 2 {
                    self.row(case, Verdict::Fail, &format!("{new} clients signed in during the run, not the run's 2"));
                } else if !active.is_empty() {
                    self.row(case, Verdict::Fail, &format!("used during the run: {}", active.join(", ")));
                } else {
                    self.row(case, Verdict::Pass, "");
                }
            }
            Ok(None) => self.row(case, Verdict::Fail, "the session list went away during the run"),
            Err(e) => self.row(case, Verdict::Fail, &e),
        }
    }

    /// Where `rel` (inside the run's folder) is on a client.
    pub fn path(&self, who: Who, rel: &str) -> PathBuf {
        let base = self.client(who).folder.join(&self.cloud);
        if rel.is_empty() { base } else { base.join(rel) }
    }

    /// One `sync --once --json` pass on a client, after the settle wait. Its output is kept as
    /// `passes/NN-<client>.json`.
    pub fn sync(&mut self, who: Who) {
        std::thread::sleep(self.settle);
        let name = format!("{:02}-{}", self.passes.len() + 1, self.client(who).name.to_lowercase());
        let out = self.dir.join("passes").join(format!("{name}.json"));
        let code = self.client(who).run(&["sync", "--once", "--json"], &out, PASS_TIMEOUT).unwrap_or_else(|e| {
            eprintln!("e2e-sync: {e}");
            -1
        });
        self.passes.push((name, code));
    }

    /// A pass on `first`, then one on the other client: a change made on `first` reaches the other.
    pub fn pair(&mut self, first: Who) {
        self.sync(first);
        self.sync(first.other());
    }

    /// The last pass's JSON on a client.
    pub fn last_pass(&self, who: Who) -> serde_json::Value {
        let name = self.client(who).name.to_lowercase();
        let last = self.passes.iter().rev().find(|(p, _)| p.ends_with(&name));
        last.and_then(|(p, _)| fs::read_to_string(self.dir.join("passes").join(format!("{p}.json"))).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null)
    }

    /// A command that must work (`--json` is added); its JSON. Output kept under `log/`.
    pub fn cli_json(&mut self, who: Who, args: &[&str]) -> Result<serde_json::Value, String> {
        let (code, out) = self.cli(who, args)?;
        if code != 0 {
            return Err(format!("`craftcloud {}` on {} exited with {code}", args.join(" "), self.client(who).name));
        }
        serde_json::from_str(&out).map_err(|e| format!("`craftcloud {}`: not JSON ({e})", args.join(" ")))
    }

    /// A command's exit code and stdout (`--json` is added).
    fn cli(&mut self, who: Who, args: &[&str]) -> Result<(i32, String), String> {
        self.calls += 1;
        let name = format!("{:03}-{}-{}.json", self.calls, self.client(who).name.to_lowercase(), args.first().unwrap_or(&""));
        let out = self.dir.join("log").join(name);
        let mut all = args.to_vec();
        all.push("--json");
        let code = self.client(who).run(&all, &out, CMD_TIMEOUT)?;
        Ok((code, fs::read_to_string(&out).unwrap_or_default()))
    }

    /// Trashed items of this run (its folder, or CraftCloud's kept versions of its files).
    pub fn trashed(&self) -> Result<Vec<api::Trashed>, String> {
        Ok(self.api.trash()?.into_iter().filter(|t| of_run(&self.cloud, &t.path)).collect())
    }

    /// Everything the run started goes: the cloud folder and its kept versions (to the trash, then
    /// for good), the two clients' sign-ins, the clients' folders and tokens. With `keep`, the
    /// folders stay (the sign-ins still end).
    fn cleanup(&mut self, keep: bool, before: Option<&[Session]>, revoke: bool) {
        let mut notes = Vec::new();
        if !keep && self.laptop.signed_in {
            notes.push(self.remove_cloud_folder());
        }
        let mut signed_in = 0;
        for who in [Who::Laptop, Who::Studio] {
            if self.client(who).signed_in {
                signed_in += 1;
                let _ = self.cli(who, &["logout"]);
            }
        }
        if signed_in > 0 {
            self.logout_ended(before, revoke);
        }
        self.api.sign_out();
        if !keep {
            for root in [&self.laptop.root, &self.studio.root] {
                if let Err(e) = fs::remove_dir_all(root) {
                    notes.push(format!("couldn't remove {}: {e}", root.display()));
                }
            }
        }
        let detail = if keep {
            format!("kept: cloud folder {} and {}", self.cloud, self.dir.display())
        } else {
            notes.retain(|n| !n.is_empty());
            notes.join("; ")
        };
        let verdict = if keep || detail.is_empty() { Verdict::Pass } else { Verdict::Warn };
        self.row("cleanup", verdict, &detail);
    }

    /// `craftcloud logout` says it ends the sign-in on the server too. With an admin account the
    /// run checks that: a client session that signed in during the run and is still active is a
    /// FAIL, and the run then ends it itself, so the next run doesn't find it signed in (unless other
    /// clients were allowed: one of theirs may have signed in meanwhile, and is left alone).
    fn logout_ended(&mut self, before: Option<&[Session]>, revoke: bool) {
        let case = "the clients' logout ended their sign-ins on the server";
        let Some(before) = before else { return };
        let now = match self.api.sessions() {
            Ok(Some(now)) => now,
            Ok(None) => return self.row(case, Verdict::Fail, "the session list went away during the run"),
            Err(e) => return self.row(case, Verdict::Fail, &e),
        };
        let left: Vec<&Session> = now.iter().filter(|s| s.origin == "device" && !before.iter().any(|b| b.id == s.id)).collect();
        if left.is_empty() {
            return self.row(case, Verdict::Pass, "");
        }
        let after = if revoke {
            let ended = left.iter().filter(|s| self.api.revoke(&s.id)).count();
            format!("the run ended {ended} through the admin API")
        } else {
            "left signed in, since other clients were allowed".to_owned()
        };
        let detail = format!("{} client sessions signed in during the run are still signed in after `craftcloud logout`; {after}", left.len());
        self.row(case, Verdict::Fail, &detail);
    }

    /// Trashes the run's folder (and its kept versions), then deletes those items for good. A note
    /// on what was left, or empty.
    fn remove_cloud_folder(&mut self) -> String {
        let cloud = self.cloud.clone();
        let versions = format!(".craftcloud/versions/{cloud}");
        let _ = self.cli(Who::Laptop, &["rm", &cloud]);
        if matches!(self.cli(Who::Laptop, &["ls", &versions]), Ok((0, _))) {
            let _ = self.cli(Who::Laptop, &["rm", &versions]);
        }
        match self.trashed() {
            Ok(items) => {
                for item in &items {
                    self.api.purge(&item.id);
                }
                match self.trashed() {
                    Ok(left) if left.is_empty() => String::new(),
                    Ok(left) => format!("{} items of the run left in the trash", left.len()),
                    Err(e) => e,
                }
            }
            Err(e) => e,
        }
    }

    /// Prints the table (and writes it to `report.md`); an error when a case failed.
    fn report(&self) -> Result<(), String> {
        let mut table = String::from("| # | Case | Result | Detail |\n|---|---|---|---|\n");
        for (i, row) in self.rows.iter().enumerate() {
            let mark = match row.verdict {
                Verdict::Pass => "PASS",
                Verdict::Fail => "**FAIL**",
                Verdict::Warn => "WARN",
            };
            let _ = writeln!(table, "| {} | {} | {mark} | {} |", i + 1, row.case, row.detail.replace('|', "\\|"));
        }
        let count = |v: Verdict| self.rows.iter().filter(|r| r.verdict == v).count();
        let (failed, warned) = (count(Verdict::Fail), count(Verdict::Warn));
        let _ = writeln!(
            table,
            "\n{} passed, {failed} failed, {warned} warnings; {} sync passes, kept in {}",
            count(Verdict::Pass),
            self.passes.len(),
            self.dir.join("passes").display()
        );
        let _ = fs::write(self.dir.join("report.md"), &table);
        println!("\n{table}");
        if failed == 0 { Ok(()) } else { Err(format!("{failed} checks failed")) }
    }
}

/// A session for the table: what it is and when it was last seen.
fn describe(s: &Session) -> String {
    let what = if s.user_agent.is_empty() { s.origin.as_str() } else { s.user_agent.as_str() };
    format!("{what} (signed in {}, last seen {})", s.created_at, s.last_seen_at)
}

/// What `PROGRAM --version` says (`craftcloud 0.1.1`), for the table.
fn program_version(bin: &Path) -> String {
    std::process::Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "an unknown version".to_owned())
}

/// Other `craftcloud` processes on this computer (Linux: from `/proc`; elsewhere none are seen).
fn other_local_clients() -> Vec<String> {
    let Ok(dir) = fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let comm = fs::read_to_string(e.path().join("comm")).ok()?;
            comm.contains("craftcloud").then(|| format!("{} (pid {pid})", comm.trim()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::options;

    fn bins(args: &[&str]) -> (Option<PathBuf>, Option<PathBuf>) {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let o = options(&args).unwrap().unwrap();
        let (laptop, studio) = o.bins();
        (laptop.cloned(), studio.cloned())
    }

    /// A mixed pair: each client can run a program of its own (an older release on one side).
    #[test]
    fn each_client_can_run_its_own_program() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(bins(&[]), (None, None), "no option: this tree's build on both");
        assert_eq!(bins(&["--bin", "new"]), (p("new"), p("new")), "--bin: the same on both");
        assert_eq!(bins(&["--bin-laptop", "old", "--bin-studio", "new"]), (p("old"), p("new")));
        assert_eq!(bins(&["--bin", "new", "--bin-studio", "old"]), (p("new"), p("old")), "a side's own option wins over --bin");
        assert_eq!(bins(&["--bin-laptop", "old"]), (p("old"), None), "the other side: this tree's build");
        assert!(options(&["--bin-studio".to_owned()]).is_err(), "needs a value");
    }

    /// T-060: a pass sooner than the 2 s a changed file is held would skip it; more than a minute
    /// isn't a settle wait. Both are refused.
    #[test]
    fn settle_waits_outside_2_to_60_seconds_are_refused() {
        let settle = |s: &str| options(&["--settle".to_owned(), s.to_owned()]);
        for bad in ["2", "1.5", "0", "-3", "61", "soon"] {
            assert!(settle(bad).is_err(), "{bad}");
        }
        assert_eq!(settle("2.5").unwrap().unwrap().settle, Duration::from_millis(2500));
        assert_eq!(settle("60").unwrap().unwrap().settle, Duration::from_secs(60));
    }

    /// Only the run's own folder and its kept versions count as the run's trash (#38).
    #[test]
    fn only_the_runs_own_trash_is_its_own() {
        let run = "e2e-sync-123";
        assert!(super::of_run(run, "john/e2e-sync-123"));
        assert!(super::of_run(run, "john/e2e-sync-123/a.txt"));
        assert!(super::of_run(run, "john/.craftcloud/versions/e2e-sync-123/a.txt/1"));
        assert!(!super::of_run(run, "john/e2e-sync-1234"), "another run's");
        assert!(!super::of_run(run, "john/Docs/e2e-sync-123"));
        assert!(!super::of_run(run, "john/.craftcloud/versions/Docs/a.txt/1"));
    }
}
