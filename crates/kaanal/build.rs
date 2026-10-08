use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = Path::new(&manifest_dir).join("../..");
    let git_dir = root.join(".git");
    let hooks_dir = root.join(".githooks");

    if git_dir.exists() && hooks_dir.is_dir() {
        let _ = Command::new("git")
            .current_dir(&root)
            .args(["config", "core.hooksPath", ".githooks"])
            .status();
    }
}
