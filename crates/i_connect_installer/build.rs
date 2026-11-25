use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn copy_or_empty(src: &Path, dst: &Path) {
    if src.exists() {
        let _ = fs::copy(src, dst);
    } else {
        println!(
            "cargo:warning=Payload {} not built yet; embedding empty placeholder for check build",
            src.display()
        );
        let _ = fs::write(dst, b"");
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let server_bin = root.join("bin").join("I_Connect_Server.exe");
    let uninst_bin = root.join("bin").join("uninstall.exe");
    let ico_file = root.join("assets").join("pc_icon.ico");

    println!("cargo:rerun-if-changed=resource.rc");
    println!("cargo:rerun-if-changed={}", server_bin.display());
    println!("cargo:rerun-if-changed={}", uninst_bin.display());
    println!("cargo:rerun-if-changed={}", ico_file.display());

    copy_or_empty(&server_bin, &out_dir.join("I_Connect_Server.exe"));
    copy_or_empty(&uninst_bin, &out_dir.join("uninstall.exe"));
    copy_or_empty(&ico_file, &out_dir.join("pc_icon.ico"));

    if ico_file.exists() {
        embed_resource::compile("resource.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
