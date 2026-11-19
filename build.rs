//! Embeds the built single-file UI bundle into the binary.
//!
//! The UI is a Vite project under `web/` that inlines everything into one
//! `index.html`. We cannot `include_str!` it directly from `src/`, so this build
//! script copies the path into `OUT_DIR` and regenerates when the bundle changes.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let index = manifest.join("web").join("dist").join("index.html");
    println!("cargo:rerun-if-changed=web/dist/index.html");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("ui.rs");
    let source = if index.exists() {
        format!(
            "/// Single-file UI bundle (embedded from web/dist/index.html).\npub const INDEX_HTML: &str = include_str!({:?});\n",
            index.to_string_lossy()
        )
    } else {
        "/// The web UI was not built; the server answers with a build hint.\npub const INDEX_HTML: &str = \"\";\n".to_string()
    };
    std::fs::write(out, source).expect("write ui.rs");
}
