use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=resource.rc");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=../../assets/pc_icon.ico");
    if Path::new("../../assets/pc_icon.ico").exists() {
        embed_resource::compile("resource.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
