fn main() {
    println!("cargo:rerun-if-env-changed=BB_IMAGER_SKIP_ADMIN_MANIFEST");

    // The production executable must be elevated to open raw disks on Windows. Unit tests do not
    // touch raw disks, and embedding the same manifest makes Windows launch the test harness via
    // UAC instead of returning its output to Cargo. CI and local test jobs may opt out explicitly;
    // normal debug/release/package builds keep the administrator manifest unchanged.
    if std::env::var_os("BB_IMAGER_SKIP_ADMIN_MANIFEST").is_some() {
        return;
    }

    embed_resource::compile(
        "assets/packages/windows/gui-manifest.rc",
        embed_resource::NONE,
    )
    .manifest_required()
    .unwrap();
}
