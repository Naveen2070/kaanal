//! Sets `cfg(kaanal_ui)` when the built Studio bundle is in `ui-dist/`. Never runs Node or pnpm:
//! the bundle is built by `ui/` (see docs/web/studio.md) and copied in before `cargo build`.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(kaanal_ui)");
    println!("cargo:rerun-if-changed=ui-dist");
    let dir = Path::new("ui-dist");
    if ["app.js", "app.css", "index.html"]
        .iter()
        .all(|f| dir.join(f).is_file())
    {
        println!("cargo:rustc-cfg=kaanal_ui");
    } else {
        println!(
            "cargo:warning=no Studio bundle in crates/kaanal-server/ui-dist: /__kaanal/studio serves the status page (build ui/ first, see docs/web/studio.md)"
        );
    }
}
