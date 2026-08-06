fn main() {
    println!("cargo:rerun-if-changed=assets/helper-manifest.rc");
    println!("cargo:rerun-if-changed=assets/helper.exe.manifest");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    embed_resource::compile("assets/helper-manifest.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the WinUSB helper manifest");
}
