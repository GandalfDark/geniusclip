// Embeds the NSIS setup built by `tauri build` (path in GC_SETUP_PAYLOAD) and
// the app version. Without a payload the installer builds in demo mode: the
// window works, and "installing" is simulated (for designing the UI).
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=GC_SETUP_PAYLOAD");
    println!("cargo:rerun-if-changed=setup.rc");
    println!("cargo:rerun-if-changed=setup.manifest");
    println!("cargo:rerun-if-changed=../../src-tauri/tauri.conf.json");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let payload = match std::env::var("GC_SETUP_PAYLOAD") {
        Ok(p) if !p.is_empty() => {
            let p = PathBuf::from(p);
            assert!(p.is_file(), "GC_SETUP_PAYLOAD does not exist: {}", p.display());
            println!("cargo:rerun-if-changed={}", p.display());
            p
        }
        _ => {
            let empty = out.join("empty-payload.bin");
            std::fs::write(&empty, []).unwrap();
            empty
        }
    };
    println!("cargo:rustc-env=GC_SETUP_PAYLOAD_PATH={}", payload.display());

    let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("../../src-tauri/tauri.conf.json").unwrap()).unwrap();
    println!("cargo:rustc-env=GC_VERSION={}", conf["version"].as_str().unwrap());

    embed_resource::compile("setup.rc", embed_resource::NONE).manifest_required().unwrap();
}
