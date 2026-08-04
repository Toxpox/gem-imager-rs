fn main() {
    println!("cargo:rerun-if-env-changed=BB_IMAGER_SKIP_ADMIN_MANIFEST");
    // The application icon is embedded by the Windows resource compiler. Track both the
    // resource script and the ICO so changing branding always updates the executable icon.
    println!("cargo:rerun-if-changed=assets/packages/windows/gui-manifest.rc");
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");

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
