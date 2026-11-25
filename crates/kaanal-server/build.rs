//! Generates Studio's asset table from the Vite manifest in `ui-dist/.vite/manifest.json` and sets
//! `cfg(kaanal_ui)` when there is one (R1.106). Embeds exactly the files the manifest references;
//! a referenced file that is missing fails the build. Never runs Node or pnpm: the bundle is
//! built by `ui/` (`pnpm build` there; docs/source/studio-ui.md) and copied in before `cargo build`.

#[path = "src/control/manifest.rs"]
mod manifest;

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(kaanal_ui)");
    println!("cargo:rerun-if-changed=ui-dist");
    let root = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let dist = PathBuf::from(root).join("ui-dist");
    match manifest::generate(&dist) {
        Ok(Some((code, files))) => {
            // A directory watch misses a deleted file; naming each one reruns this script, which
            // then fails with the manifest's own message.
            for f in files {
                println!("cargo:rerun-if-changed={}", f.display());
            }
            let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
            std::fs::write(out.join("studio_bundle.rs"), code)
                .expect("cannot write studio_bundle.rs to OUT_DIR");
            println!("cargo:rustc-cfg=kaanal_ui");
        }
        Ok(None) => println!(
            "cargo:warning=no Studio bundle in crates/kaanal-server/ui-dist (no .vite/manifest.json): /__kaanal/studio serves the status page (build ui/ first: cd ui && pnpm install && pnpm build, then rebuild; see https://kaanal.naveenr.in/docs/start/install/)"
        ),
        Err(e) => panic!("Studio bundle in {}: {e}", dist.display()),
    }
}
