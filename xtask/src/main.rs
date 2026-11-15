//! Repository tasks, run as `cargo xtask <command>` (the alias is in .cargo/config.toml).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bump_apps;
mod fetch_apps;
mod manifest;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

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
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => cmd_ci(),
        Some("wasm") => cmd_wasm(),
        Some("web") => cmd_web(),
        Some("fetch-apps") => fetch_apps::cmd(args.get(1..).unwrap_or_default()),
        Some("bump-apps") => bump_apps::cmd(args.get(1..).unwrap_or_default()),
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

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, fn() -> Result<(), String>);
    let steps: [Step; 4] = [
        ("fmt", || cargo(&["fmt", "--all", "--", "--check"])),
        ("clippy", || cargo(&["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"])),
        ("test", || cargo(&["test", "--workspace", "--locked"])),
        ("wasm", cmd_wasm),
    ];
    eprintln!("ci: checking {}", root().display());
    let mut done = Vec::new();
    for (name, step) in steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = step() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(name);
    }
    println!("\nCI summary: all {} steps passed ({}) in {}", done.len(), done.join(", "), root().display());
    Ok(())
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
