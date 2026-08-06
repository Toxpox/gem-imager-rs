//! Read-only diagnostic for the T3 ROM DFU Windows driver state.

fn main() {
    let state = gem_winusb::probe();
    println!("PnP state: {state:?}");
    if matches!(
        state,
        gem_winusb::DriverState::ReadyWinUsb | gem_winusb::DriverState::ReadyExternal { .. }
    ) {
        match gem_winusb::verify_dfu_transport() {
            Ok(()) => println!("libusb transport: open OK"),
            Err(error) => {
                eprintln!("libusb transport: FAILED: {error}");
                std::process::exit(2);
            }
        }
    }
}
