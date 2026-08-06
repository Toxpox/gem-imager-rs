use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-env-changed=BB_IMAGER_SKIP_ADMIN_MANIFEST");
    println!("cargo:rerun-if-env-changed=PROFILE");
    // The application icon is embedded by the Windows resource compiler. Track both the
    // resource script and the ICO so changing branding always updates the executable icon.
    println!("cargo:rerun-if-changed=assets/packages/windows/gui-manifest.rc");
    println!("cargo:rerun-if-changed=assets/packages/windows/gui-as-invoker-manifest.rc");
    println!("cargo:rerun-if-changed=assets/packages/windows/gui-as-invoker.exe.manifest");
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");

    embed_winusb_helper_hash();

    // The production executable must currently be elevated to open raw disks on Windows. Build
    // scripts cannot distinguish Cargo's test harness from the package binary at link time, so an
    // admin manifest in the debug profile also lands in every unit-test executable and makes a
    // normal `cargo test` fail with Windows error 740. Release/package builds retain the existing
    // manifest; debug builds and tests are asInvoker. The WinUSB helper has its own independent
    // requireAdministrator manifest, so this distinction cannot bypass driver-install elevation.
    let resource = if std::env::var_os("BB_IMAGER_SKIP_ADMIN_MANIFEST").is_none()
        && std::env::var_os("PROFILE").as_deref() == Some(std::ffi::OsStr::new("release"))
    {
        "assets/packages/windows/gui-manifest.rc"
    } else {
        "assets/packages/windows/gui-as-invoker-manifest.rc"
    };

    embed_resource::compile(resource, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}

/// Bind the GUI to the exact helper produced immediately before it by the Windows packaging
/// target. This is required for a portable directory, where sibling files are user-writable.
fn embed_winusb_helper_hash() {
    if std::env::var_os("CARGO_FEATURE_DFU_DRIVER_MVP").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest_dir
        .parent()
        .expect("GUI crate has a workspace parent");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));
    let target = std::env::var("TARGET").expect("Cargo supplies TARGET");
    let profile = std::env::var("PROFILE").expect("Cargo supplies PROFILE");

    let candidates = [
        target_dir
            .join(&target)
            .join(&profile)
            .join("bb-winusb-helper.exe"),
        target_dir.join(&profile).join("bb-winusb-helper.exe"),
    ];
    let helper = candidates.iter().find(|path| path.is_file());
    let hash = match helper {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", path.display());
            sha256(path).unwrap_or_else(|error| {
                panic!("failed to hash WinUSB helper {}: {error}", path.display())
            })
        }
        None if profile == "release" && target.contains("windows") => panic!(
            "dfu-driver-mvp release builds require bb-winusb-helper.exe first; use the Windows packaging target"
        ),
        None => String::new(),
    };
    println!("cargo:rustc-env=BB_WINUSB_HELPER_SHA256={hash}");
}

fn sha256(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
