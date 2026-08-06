#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use gem_winusb::{
        HELPER_EXIT_ALREADY_READY, HELPER_EXIT_INSTALLED, HELPER_EXIT_INVALID_COMMAND,
        InstallOutcome,
    };

    let mut args = std::env::args_os();
    let _executable = args.next();
    let command = args.next();
    if command.as_deref() != Some(std::ffi::OsStr::new("install-t3-rom-dfu"))
        || args.next().is_some()
    {
        std::process::exit(HELPER_EXIT_INVALID_COMMAND);
    }

    let exit_code = match gem_winusb::install_t3_rom_dfu() {
        Ok(InstallOutcome::Installed) => HELPER_EXIT_INSTALLED,
        Ok(InstallOutcome::AlreadyReady) => HELPER_EXIT_ALREADY_READY,
        Err(error) => error.helper_exit_code(),
    };
    std::process::exit(exit_code);
}

#[cfg(not(windows))]
fn main() {
    // The package remains cross-compilable for workspace CI, but it has no non-Windows action.
}
