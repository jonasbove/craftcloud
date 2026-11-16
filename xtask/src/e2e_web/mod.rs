//! `cargo xtask e2e-web`: the web frontend in a real browser against a real server
//! (docs/TESTING.md).
//!
//! Headless Chromium, driven through chromedriver, runs the WebAssembly app the server serves and
//! goes through first-run setup, sign-out and sign-in, an upload, a share link opened signed out,
//! a device approval and an admin change. The app draws on a canvas, so the run works it with
//! the keyboard (Tab, typing, Enter), as a person without a mouse would, and checks every step
//! where it shows: the server's API, the browser's cookie jar and its network log. Never by
//! pixels; screenshots are only for a person reading a failure.

mod api;
mod browser;
mod cases;
mod webdriver;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use api::Api;
use browser::{Browser, Driver, Programs};

const USAGE: &str = "\
usage: cargo xtask e2e-web [--server URL] [--shots] [--dir PATH]

The web frontend in headless Chromium against a fresh CraftCloud server (no owner yet): first-run
setup, sign-out and sign-in, an upload, a share link, a device approval and the admin CSRF check.
Needs chromedriver and Chromium or Chrome of the same version (docs/TESTING.md).

  --server URL   the server (default: CRAFTCLOUD_TEST_SERVER); it must not be set up yet
  --shots        save a screenshot after every step (a failure always saves one)
  --dir PATH     where the run directory goes (default: <target>/e2e-web)

The owner is CRAFTCLOUD_TEST_USER (default john) with CRAFTCLOUD_TEST_PASSWORD (default: a new
random one, never printed). CRAFTCLOUD_E2E_CHROMEDRIVER and CRAFTCLOUD_E2E_CHROME name the
programs when they aren't on PATH.
";

struct Options {
    server: Option<String>,
    shots: bool,
    dir: Option<PathBuf>,
}

fn options(args: &[String]) -> Result<Option<Options>, String> {
    let mut o = Options { server: std::env::var("CRAFTCLOUD_TEST_SERVER").ok().filter(|s| !s.is_empty()), shots: false, dir: None };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value\n\n{USAGE}"));
        match arg.as_str() {
            "--server" => o.server = Some(value()?),
            "--dir" => o.dir = Some(PathBuf::from(value()?)),
            "--shots" => o.shots = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
        }
    }
    Ok(Some(o))
}

pub fn cmd(args: &[String], root: &Path) -> Result<(), String> {
    let Some(o) = options(args)? else { return Ok(()) };
    let server = o.server.ok_or_else(|| format!("no server: pass --server URL or set CRAFTCLOUD_TEST_SERVER\n\n{USAGE}"))?;
    let server = server.trim_end_matches('/').to_owned();
    // Find the programs before anything else, so a missing driver or browser says so at once.
    let programs = Programs::find()?;
    let api = Api::new(&server);
    // First-run setup is the first case and everything after builds on it.
    if api.initialized()? {
        return Err(format!("{server} already has an owner: e2e-web goes through first-run setup, so it needs a fresh server (docs/TESTING.md)"));
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let dir = o.dir.unwrap_or_else(|| root.join("target").join("e2e-web")).join(secs.to_string());
    fs::create_dir_all(&dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    eprintln!("e2e-web: {server}, run directory {}", dir.display());
    eprintln!("e2e-web: {} with {}", programs.browser.display(), programs.driver.display());

    let user = std::env::var("CRAFTCLOUD_TEST_USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "john".into());
    let password = std::env::var("CRAFTCLOUD_TEST_PASSWORD").ok().filter(|p| !p.is_empty()).unwrap_or_else(random_password);
    let driver = Driver::start(&programs, &dir.join("chromedriver.log"))?;
    let browser = Browser::open(&driver, &programs, &dir, "owner", o.shots)?;
    let mut run =
        Run { api, user, password, bearer: None, owner: browser, visitor: None, programs, driver, dir, shots: o.shots, secs, rows: Vec::new() };
    cases::all(&mut run);
    run.finish()
}

/// A password for the owner: long, random (the standard library's hasher keys are seeded from
/// the system's random source), never printed.
fn random_password() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::from("E2e-");
    for i in 0..3u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        let _ = write!(out, "{:016x}", h.finish());
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Skip,
}

struct Row {
    case: String,
    verdict: Verdict,
    detail: String,
}

pub struct Run {
    pub api: Api,
    pub user: String,
    pub password: String,
    /// The owner's API tokens (a password sign-in of the run's own: access, refresh), once signed in.
    pub bearer: Option<(String, String)>,
    /// The owner's browser.
    pub owner: Browser,
    /// A second browser that never signs in (the share link's visitor).
    pub visitor: Option<Browser>,
    programs: Programs,
    driver: Driver,
    pub dir: PathBuf,
    shots: bool,
    /// The run's start, for names that don't clash with an earlier run's.
    pub secs: u64,
    rows: Vec<Row>,
}

impl Run {
    pub fn url(&self, path: &str) -> String {
        self.api.url(path)
    }

    /// Opens the second browser, with a profile of its own (no cookies).
    pub fn visitor(&mut self) -> Result<&mut Browser, String> {
        if self.visitor.is_none() {
            self.visitor = Some(Browser::open(&self.driver, &self.programs, &self.dir, "visitor", self.shots)?);
        }
        self.visitor.as_mut().ok_or_else(|| "no second browser".to_owned())
    }

    /// Records a case's result; a failure saves what each browser shows.
    pub fn row(&mut self, case: &str, result: Result<String, String>) -> bool {
        let (verdict, detail) = match result {
            Ok(detail) => (Verdict::Pass, detail),
            Err(why) => {
                let name = case.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
                let shot = self.dir.join(format!("failed-{name}-owner.png"));
                self.owner.shot_to(&shot);
                if let Some(v) = &self.visitor {
                    v.shot_to(&self.dir.join(format!("failed-{name}-visitor.png")));
                }
                (Verdict::Fail, format!("{why} (screenshot: {})", shot.display()))
            }
        };
        self.mark(case, verdict, &detail);
        verdict == Verdict::Pass
    }

    pub fn skip(&mut self, case: &str, why: &str) {
        self.mark(case, Verdict::Skip, why);
    }

    fn mark(&mut self, case: &str, verdict: Verdict, detail: &str) {
        let mark = match verdict {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Skip => "SKIP",
        };
        eprintln!("e2e-web: {mark}  {case}  ({detail})");
        self.rows.push(Row { case: case.to_owned(), verdict, detail: detail.to_owned() });
    }

    /// Ends the run's own sign-ins, prints the table (and writes it to `report.md`); an error when
    /// a case failed.
    fn finish(mut self) -> Result<(), String> {
        if let Some((access, refresh)) = self.bearer.take() {
            self.api.sign_out(&access, &refresh);
        }
        self.owner.quit();
        if let Some(v) = &mut self.visitor {
            v.quit();
        }
        let mut table = String::from("| # | Case | Result | Detail |\n|---|---|---|---|\n");
        for (i, row) in self.rows.iter().enumerate() {
            let mark = match row.verdict {
                Verdict::Pass => "PASS",
                Verdict::Fail => "**FAIL**",
                Verdict::Skip => "SKIP",
            };
            let _ = writeln!(table, "| {} | {} | {mark} | {} |", i + 1, row.case, row.detail.replace('|', "\\|"));
        }
        let count = |v: Verdict| self.rows.iter().filter(|r| r.verdict == v).count();
        let failed = count(Verdict::Fail);
        let _ = writeln!(
            table,
            "\n{} passed, {failed} failed, {} skipped; run directory {}",
            count(Verdict::Pass),
            count(Verdict::Skip),
            self.dir.display()
        );
        let _ = fs::write(self.dir.join("report.md"), &table);
        println!("\n{table}");
        if failed == 0 { Ok(()) } else { Err(format!("{failed} checks failed")) }
    }
}
