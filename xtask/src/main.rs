//! Repository tasks, run as `cargo xtask <command>` (the alias is in .cargo/config.toml).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bump_apps;
mod e2e;
mod e2e_web;
mod fetch_apps;
mod icons;
mod manifest;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use craft_xtask::runner::{self, RunOptions, Step};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci     fmt check, clippy (-D warnings), tests, and the wasm check: what CI runs
  wasm   clippy for the crates the web frontend builds, on wasm32-unknown-unknown
  web    build the web frontend with Trunk into dist/web (needs `trunk` on PATH)
  fetch-apps <apps.toml> <out-dir> [--cache DIR] [--only app,app] [--jobs N]
         download, verify (sha256), unpack and precompress the Craft web builds apps.toml pins,
         and write <out-dir>/apps.json (the Dockerfile's `apps` stage)
  bump-apps [--check] [--app <id>]...
         update apps.toml's pins (version, asset, sha256) from each app's latest GitHub release;
         --check only reports, and fails if a pin is out of date
  icons [SOURCE.png]
         render the packaging icons (Linux hicolor PNGs, the Windows .ico, the macOS .icns) from
         the logo, design/assets/logo/craftcloud-logo-1024.png by default
  e2e-sync  two scratch clients sync a fresh folder on CRAFTCLOUD_TEST_SERVER and check each other
            (create, edit, rename, move, delete to the trash, conflicts, …): docs/TESTING.md
  e2e-web   the web frontend in headless Chromium against a fresh server (setup, sign-in, upload, share
            link, device approval, admin CSRF); needs chromedriver: docs/TESTING.md
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => cmd_ci(),
        Some("wasm") => cmd_wasm(),
        Some("web") => cmd_web(),
        Some("fetch-apps") => fetch_apps::cmd(args.get(1..).unwrap_or_default()),
        Some("bump-apps") => bump_apps::cmd(args.get(1..).unwrap_or_default()),
        Some("icons") => icons::cmd(args.get(1..).unwrap_or_default(), &root()),
        Some("e2e-sync") => e2e::cmd(args.get(1..).unwrap_or_default(), &root()),
        Some("e2e-web") => e2e_web::cmd(args.get(1..).unwrap_or_default(), &root()),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace being checked: the nearest folder at or above the current one whose `Cargo.toml`
/// has a `[workspace]` table. Not the folder xtask was compiled in: with a shared target directory
/// (several worktrees), the xtask binary may have been built from another checkout.
fn root() -> PathBuf {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    here.ancestors()
        .find(|dir| std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|toml| toml.lines().any(|l| l.trim() == "[workspace]")))
        .map(Path::to_path_buf)
        .unwrap_or(here)
}

fn run(program: &str, args: &[&str], dir: &Path) -> Result<(), String> {
    eprintln!("$ {program} {}", args.join(" "));
    let status = Command::new(program).args(args).current_dir(dir).status().map_err(|e| format!("couldn't run {program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("`{program} {}` failed ({status})", args.join(" "))) }
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    run(&cargo, args, &root())
}

/// The commands `ci` runs, in order: `cargo <args>` in the workspace, and the wasm check.
const CI_CARGO_STEPS: [(&str, &[&str]); 3] = [
    ("fmt", &["fmt", "--all", "--", "--check"]),
    ("clippy", &["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]),
    ("test", &["test", "--workspace", "--locked"]),
];

/// `cargo xtask ci`, on craft-xtask's step runner (the Craft apps' gate runner): the steps in order,
/// stopping at the first failure, then the summary. The summary of a passing run names the tree it
/// checked (AGENTS.md: commit only when it says `all 4 steps passed` in the tree you meant), which
/// craft-xtask's own summary doesn't, so [`summary_naming_tree`] adds it.
fn cmd_ci() -> Result<(), String> {
    let root = root();
    eprintln!("ci: checking {}", root.display());
    let mut steps: Vec<Step<'_>> = CI_CARGO_STEPS
        .iter()
        .map(|&(name, args)| {
            let root = root.clone();
            let label = format!("cargo {}", args.join(" "));
            Step::new(name, move |log| {
                let mut c = runner::cargo(&root);
                c.args(args);
                runner::run(c, log.command(&label))
            })
        })
        .collect();
    steps.push(Step::new("wasm", |log| {
        log.command("cargo clippy (wasm32-unknown-unknown)");
        cmd_wasm()
    }));
    let mut summary = Vec::new();
    let result = runner::run_steps_to(&mut steps, &RunOptions::default(), &mut summary, &mut std::io::stderr());
    print!("{}", summary_naming_tree(&String::from_utf8_lossy(&summary), &root));
    result.map(|_| ()).map_err(|e| format!("ci failed: {e}"))
}

/// `summary` with ` in <root>` after the line that says every step passed; other lines unchanged.
fn summary_naming_tree(summary: &str, root: &Path) -> String {
    summary
        .split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches('\n');
            if body.starts_with("CI summary: all ") && body.contains(" steps passed") {
                format!("{body} in {}{}", root.display(), &line[body.len()..])
            } else {
                line.to_owned()
            }
        })
        .collect()
}

fn cmd_wasm() -> Result<(), String> {
    cargo(&[
        "clippy",
        "-p",
        "craftcloud-core",
        "-p",
        "craftcloud-ui-egui",
        "-p",
        "craftcloud-web",
        "--target",
        "wasm32-unknown-unknown",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])
}

fn cmd_web() -> Result<(), String> {
    run("trunk", &["build", "--release", "--cargo-profile", "wasm-release"], &root().join("apps").join("craftcloud-web"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passing_summary_names_the_tree_it_checked() {
        let summary = "\nCI summary: all 4 steps passed (fmt, clippy, test, wasm)\n";
        assert_eq!(
            summary_naming_tree(summary, Path::new("/work/craftcloud")),
            "\nCI summary: all 4 steps passed (fmt, clippy, test, wasm) in /work/craftcloud\n"
        );
    }

    #[test]
    fn a_failing_summary_is_left_as_it_is() {
        let summary = "\nCI summary:\n  ok    fmt\n  FAIL  clippy: cargo clippy: exited with 101\n  skip  test\n  skip  wasm\n";
        assert_eq!(summary_naming_tree(summary, Path::new("/work/craftcloud")), summary);
    }

    #[test]
    fn the_runner_prints_the_line_the_tree_is_added_to() {
        // craft-xtask's own wording; if it changes, the tree would silently drop out of the summary.
        let mut steps = vec![Step::new("fmt", |_| Ok(())), Step::new("wasm", |_| Ok(()))];
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = runner::run_steps_to(&mut steps, &RunOptions::default(), &mut out, &mut err);
        assert!(result.is_ok());
        let named = summary_naming_tree(&String::from_utf8_lossy(&out), Path::new("/t"));
        assert!(named.contains("CI summary: all 2 steps passed (fmt, wasm) in /t\n"), "{named}");
    }
}
