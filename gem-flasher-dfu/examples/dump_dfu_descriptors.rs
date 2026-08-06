//! Debug aid: dump every descriptor the T3 board publishes in DFU mode.
//!
//! Run with the board attached in DFU mode:
//!     cargo run -p gem-flasher-dfu --example dump_dfu_descriptors
//!
//! It answers one question: where does this device put its DFU functional descriptor (0x21)?

use rusb::UsbContext as _;

const DFU_CLASS: u8 = 0xfe;
const DFU_SUBCLASS: u8 = 0x01;

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Walk a descriptor blob and report any DFU functional descriptor inside it.
fn scan(label: &str, mut extra: &[u8]) {
    println!("  {label}: {} bytes [{}]", extra.len(), hex(extra));
    while extra.len() >= 2 {
        let length = usize::from(extra[0]);
        if length < 2 || length > extra.len() {
            println!("    !! malformed descriptor chain at {}", hex(extra));
            return;
        }
        if extra[1] == 0x21 {
            let size = if length >= 7 {
                u16::from_le_bytes([extra[5], extra[6]])
            } else {
                0
            };
            println!("    >> DFU functional descriptor, wTransferSize = {size}");
        }
        extra = &extra[length..];
    }
}

/// Whether any interface of this device presents the DFU class.
fn has_dfu_interface(device: &rusb::Device<rusb::Context>, configs: u8) -> bool {
    (0..configs).any(|index| {
        device.config_descriptor(index).is_ok_and(|config| {
            config.interfaces().any(|interface| {
                interface.descriptors().any(|alt| {
                    alt.class_code() == DFU_CLASS && alt.sub_class_code() == DFU_SUBCLASS
                })
            })
        })
    })
}

fn main() -> rusb::Result<()> {
    let context = rusb::Context::new()?;
    let devices = context.devices()?;
    println!("{} USB device(s) attached", devices.iter().count());

    let mut dumped = 0usize;
    for device in devices.iter() {
        let Ok(descriptor) = device.device_descriptor() else {
            continue;
        };
        let is_t3 = descriptor.vendor_id() == gem_flasher_dfu::T3_DFU_VENDOR_ID
            && descriptor.product_id() == gem_flasher_dfu::T3_DFU_PRODUCT_ID;
        let is_dfu = has_dfu_interface(&device, descriptor.num_configurations());

        // One line for everything, so an empty result is never ambiguous.
        println!(
            " - {:04x}:{:04x} bus {}{}{}",
            descriptor.vendor_id(),
            descriptor.product_id(),
            device.bus_number(),
            if is_t3 { "  <- T3 DFU ids" } else { "" },
            if is_dfu { "  <- has DFU interface" } else { "" }
        );

        if !is_t3 && !is_dfu {
            continue;
        }
        dumped += 1;

        println!(
            "device {:04x}:{:04x} bus {} port {:?} bcdDevice {} bMaxPacketSize0 {}",
            descriptor.vendor_id(),
            descriptor.product_id(),
            device.bus_number(),
            device.port_numbers()?,
            descriptor.device_version(),
            descriptor.max_packet_size()
        );

        // Opening can fail (no WinUSB driver); the descriptors are still readable without it, and
        // only the human-readable alt-setting names are lost.
        let handle = match device.open() {
            Ok(handle) => Some(handle),
            Err(e) => {
                println!("  (cannot open device: {e}; alt-setting names unavailable)");
                None
            }
        };
        let language = handle.as_ref().and_then(|handle| {
            handle
                .read_languages(std::time::Duration::from_secs(5))
                .ok()
                .and_then(|languages| languages.first().copied())
        });

        if let Some(handle) = handle.as_ref() {
            // `rusb::Version` assumes valid BCD digits. Some later T3 boot stages have previously
            // appeared in Windows as `REV_7>94`, so preserve the raw bcdDevice word as evidence
            // instead of relying only on the parsed display value.
            let mut raw_device_descriptor = [0u8; 18];
            match handle.read_control(
                0x80, // standard device-to-host request for the device recipient
                0x06, // GET_DESCRIPTOR
                0x0100,
                0,
                &mut raw_device_descriptor,
                std::time::Duration::from_secs(5),
            ) {
                Ok(length) if length >= 14 => {
                    let raw_bcd_device =
                        u16::from_le_bytes([raw_device_descriptor[12], raw_device_descriptor[13]]);
                    println!(
                        "  raw device descriptor: {} bytes [{}], bcdDevice = 0x{raw_bcd_device:04x}",
                        length,
                        hex(&raw_device_descriptor[..length])
                    );
                }
                Ok(length) => println!(
                    "  raw device descriptor too short: {length} bytes [{}]",
                    hex(&raw_device_descriptor[..length])
                ),
                Err(error) => println!("  cannot read raw device descriptor: {error}"),
            }

            println!(
                "  strings: manufacturer={:?} product={:?} serial={:?}",
                handle.read_manufacturer_string_ascii(&descriptor).ok(),
                handle.read_product_string_ascii(&descriptor).ok(),
                handle.read_serial_number_string_ascii(&descriptor).ok()
            );
        }

        for index in 0..descriptor.num_configurations() {
            let config = device.config_descriptor(index)?;
            println!(" config #{}", config.number());
            scan("config extra", config.extra());

            for interface in config.interfaces() {
                for alt in interface.descriptors() {
                    let name = handle
                        .as_ref()
                        .zip(language)
                        .and_then(|(handle, lang)| {
                            handle
                                .read_interface_string(
                                    lang,
                                    &alt,
                                    std::time::Duration::from_secs(5),
                                )
                                .ok()
                        })
                        .unwrap_or_else(|| "<no string>".to_owned());
                    println!(
                        "  interface {} alt {} class {:02x}/{:02x} name {name:?}{}",
                        alt.interface_number(),
                        alt.setting_number(),
                        alt.class_code(),
                        alt.sub_class_code(),
                        if alt.class_code() == DFU_CLASS && alt.sub_class_code() == DFU_SUBCLASS {
                            "  [DFU]"
                        } else {
                            ""
                        }
                    );
                    scan("alt extra", alt.extra());
                }
            }
        }
    }

    if dumped == 0 {
        println!(
            "\nNo DFU device found. The board is not in DFU mode right now — put it in DFU mode \
             (re-plug, or hold the boot button while powering on) and run this again."
        );
    }
    Ok(())
}
