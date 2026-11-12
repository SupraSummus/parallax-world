//! The code's version, for the panel and reports to name (PARALLAX_VERSION):
//! git's commit, `-dirty` with changes not committed; `unknown` without git.
//! Built again as the commit, what is staged or the sources change.

use std::process::Command;

fn main() {
    let version = Command::new("git")
        .args(["describe", "--always", "--dirty", "--abbrev=10"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or("unknown".to_string(), |v| v.trim().to_string());
    println!("cargo:rustc-env=PARALLAX_VERSION={version}");
    let git = Command::new("git").args(["rev-parse", "--absolute-git-dir"]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok());
    if let Some(git) = git.map(|g| g.trim().to_string()).filter(|g| !g.is_empty()) {
        for file in ["HEAD", "index", "packed-refs"] {
            println!("cargo:rerun-if-changed={git}/{file}");
        }
        if let Some(head) =
            std::fs::read_to_string(format!("{git}/HEAD")).ok().and_then(|h| h.strip_prefix("ref: ").map(|r| r.trim().to_string()))
        {
            println!("cargo:rerun-if-changed={git}/{head}");
        }
    }
    for sources in ["src", "../bake/src", "../worldgen/src", "../worlds/src", "../worlds/data"] {
        println!("cargo:rerun-if-changed={sources}");
    }
}
