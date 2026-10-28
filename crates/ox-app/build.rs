//! Embeds the app icon, version info and manifest (DPI awareness, long paths,
//! modern common controls) into the Windows executable.

fn main() {
    for file in ["oxplorer.rc", "oxplorer.ico", "oxplorer.manifest"] {
        println!("cargo:rerun-if-changed=assets/{file}");
    }
    match embed_resource::compile("assets/oxplorer.rc", embed_resource::NONE) {
        embed_resource::CompilationResult::NotAttempted(why) => {
            // E.g. `cargo check --target x86_64-pc-windows-gnu` on Linux without windres.
            println!("cargo:warning=Windows resources not embedded: {why}");
        }
        result => {
            if let Err(e) = result.manifest_optional() {
                panic!("{e}");
            }
        }
    }
}
