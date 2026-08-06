//! Read-only diagnostic for the T3 ROM DFU Windows driver state.

fn main() {
    let state = bb_winusb::probe();
    println!("PnP state: {state:?}");
    if matches!(
        state,
        bb_winusb::DriverState::ReadyWinUsb | bb_winusb::DriverState::ReadyExternal { .. }
    ) {
        match bb_winusb::verify_dfu_transport() {
            Ok(()) => println!("libusb transport: open OK"),
            Err(error) => {
                eprintln!("libusb transport: FAILED: {error}");
                std::process::exit(2);
            }
        }
    }
}
