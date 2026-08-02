use std::time::{Duration, Instant};

use bb_helper::cancel::CancellationToken;
use rusb::UsbContext as _;

use crate::{
    Error, Result,
    model::{DfuDevice, DfuState, DfuStatus, TransportError, TransportErrorKind, UsbPath},
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const ENUMERATE_INTERVAL: Duration = Duration::from_millis(250);
const DFU_CLASS: u8 = 0xfe;
const DFU_SUBCLASS: u8 = 0x01;
const DFU_FUNCTIONAL_DESCRIPTOR: u8 = 0x21;

const REQUEST_DETACH: u8 = 0;
const REQUEST_DNLOAD: u8 = 1;
const REQUEST_GET_STATUS: u8 = 3;
const REQUEST_CLEAR_STATUS: u8 = 4;
const REQUEST_ABORT: u8 = 6;

pub trait DfuTransport {
    fn enumerate(&mut self, vendor_id: u16, product_id: u16) -> Result<Vec<DfuDevice>>;
    fn claim(&mut self, device: &DfuDevice, alt_setting: &str) -> Result<()>;
    fn release(&mut self) -> Result<()>;
    fn transfer_size(&mut self) -> Result<usize>;
    fn status(&mut self) -> Result<DfuStatus>;
    fn clear_status(&mut self) -> Result<()>;
    fn abort(&mut self) -> Result<()>;
    fn download_chunk(&mut self, block: u16, data: &[u8]) -> Result<usize>;
    fn finish_download(&mut self, block: u16) -> Result<()>;
    fn detach(&mut self, timeout: Duration) -> Result<()>;
    fn reset(&mut self) -> Result<()>;

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    /// Wait for one exact physical device and alt-setting. Address and serial changes are allowed;
    /// moving to another port is not. A legacy one-component path is rejected if it matches more
    /// than one full topology path.
    fn wait_for_alt(
        &mut self,
        vendor_id: u16,
        product_id: u16,
        path: &UsbPath,
        alt_setting: &str,
        deadline: Instant,
        cancel: Option<&CancellationToken>,
    ) -> Result<DfuDevice> {
        loop {
            crate::check_cancel(cancel)?;
            let matching = self
                .enumerate(vendor_id, product_id)?
                .into_iter()
                .filter(|device| path.matches(&device.path))
                .collect::<Vec<_>>();

            if matching.len() > 1 {
                return Err(Error::AmbiguousDevice {
                    vendor_id,
                    product_id,
                    count: matching.len(),
                });
            }

            if let Some(device) = matching.into_iter().next()
                && device.alt_settings.iter().any(|alt| alt == alt_setting)
            {
                return Ok(device);
            }

            if Instant::now() >= deadline {
                return Err(Error::ReconnectTimeout {
                    alt_setting: alt_setting.to_owned(),
                    path: path.clone(),
                });
            }
            self.sleep(ENUMERATE_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
}

struct ClaimedSession {
    handle: rusb::DeviceHandle<rusb::Context>,
    interface: u8,
    transfer_size: usize,
}

pub struct RusbTransport {
    session: Option<ClaimedSession>,
}

impl RusbTransport {
    pub const fn new() -> Self {
        Self { session: None }
    }

    fn session(&self) -> Result<&ClaimedSession> {
        self.session.as_ref().ok_or(Error::NoClaimedInterface)
    }

    fn map_usb(error: rusb::Error) -> Error {
        Error::Transport(TransportError::new(
            match error {
                rusb::Error::NoDevice => TransportErrorKind::Disconnected,
                rusb::Error::Io => TransportErrorKind::Io,
                rusb::Error::Timeout => TransportErrorKind::Timeout,
                rusb::Error::Pipe => TransportErrorKind::Pipe,
                rusb::Error::Access => TransportErrorKind::Access,
                rusb::Error::Busy => TransportErrorKind::Busy,
                rusb::Error::InvalidParam => TransportErrorKind::InvalidParam,
                rusb::Error::NoMem => TransportErrorKind::NoMem,
                rusb::Error::NotSupported => TransportErrorKind::NotSupported,
                _ => TransportErrorKind::Other,
            },
            error.to_string(),
        ))
    }
}

impl Default for RusbTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl DfuTransport for RusbTransport {
    fn enumerate(&mut self, vendor_id: u16, product_id: u16) -> Result<Vec<DfuDevice>> {
        let context = rusb::Context::new().map_err(Self::map_usb)?;
        let devices = context.devices().map_err(Self::map_usb)?;
        let mut found = Vec::new();

        for device in devices.iter() {
            let descriptor = device.device_descriptor().map_err(Self::map_usb)?;
            if descriptor.vendor_id() != vendor_id || descriptor.product_id() != product_id {
                continue;
            }

            let path = UsbPath::new(
                device.bus_number(),
                device.port_numbers().map_err(Self::map_usb)?,
            )
            .map_err(|message| Error::InvalidDeviceIdentity(message.to_owned()))?;
            let handle = device.open().map_err(Self::map_usb)?;
            let languages = handle.read_languages(CONTROL_TIMEOUT).unwrap_or_default();
            let language = languages.first().copied();
            let manufacturer = language.and_then(|lang| {
                handle
                    .read_manufacturer_string(lang, &descriptor, CONTROL_TIMEOUT)
                    .ok()
            });
            let product = language.and_then(|lang| {
                handle
                    .read_product_string(lang, &descriptor, CONTROL_TIMEOUT)
                    .ok()
            });
            let serial = language.and_then(|lang| {
                handle
                    .read_serial_number_string(lang, &descriptor, CONTROL_TIMEOUT)
                    .ok()
            });
            let config = device.active_config_descriptor().map_err(Self::map_usb)?;
            let mut alt_settings = Vec::new();
            for interface in config.interfaces() {
                for alt in interface.descriptors() {
                    if alt.class_code() != DFU_CLASS || alt.sub_class_code() != DFU_SUBCLASS {
                        continue;
                    }
                    if let Some(lang) = language
                        && let Ok(name) = handle.read_interface_string(lang, &alt, CONTROL_TIMEOUT)
                    {
                        alt_settings.push(name);
                    }
                }
            }
            alt_settings.sort();
            alt_settings.dedup();
            if alt_settings.is_empty() {
                continue;
            }

            let name = match (manufacturer, product) {
                (Some(manufacturer), Some(product)) => format!("{manufacturer}, {product}"),
                (_, Some(product)) => product,
                _ => format!("DFU device {vendor_id:04x}:{product_id:04x}"),
            };
            found.push(DfuDevice {
                vendor_id,
                product_id,
                path,
                address: device.address(),
                serial,
                alt_settings,
                name,
            });
        }
        Ok(found)
    }

    fn claim(&mut self, expected: &DfuDevice, alt_setting: &str) -> Result<()> {
        if self.session.is_some() {
            return Err(Error::InterfaceAlreadyClaimed);
        }
        let context = rusb::Context::new().map_err(Self::map_usb)?;
        let devices = context.devices().map_err(Self::map_usb)?;
        for device in devices.iter() {
            let descriptor = device.device_descriptor().map_err(Self::map_usb)?;
            let path = match device.port_numbers() {
                Ok(ports) => UsbPath {
                    bus: device.bus_number(),
                    ports,
                },
                Err(_) => continue,
            };
            if descriptor.vendor_id() != expected.vendor_id
                || descriptor.product_id() != expected.product_id
                || path != expected.path
            {
                continue;
            }

            let handle = device.open().map_err(Self::map_usb)?;
            let languages = handle
                .read_languages(CONTROL_TIMEOUT)
                .map_err(Self::map_usb)?;
            let language = *languages.first().ok_or_else(|| {
                Error::InvalidDeviceIdentity("device exposes no USB string language".to_owned())
            })?;
            let config = device.active_config_descriptor().map_err(Self::map_usb)?;
            for interface in config.interfaces() {
                for alt in interface.descriptors() {
                    if alt.class_code() != DFU_CLASS || alt.sub_class_code() != DFU_SUBCLASS {
                        continue;
                    }
                    let Ok(name) = handle.read_interface_string(language, &alt, CONTROL_TIMEOUT)
                    else {
                        continue;
                    };
                    if name != alt_setting {
                        continue;
                    }

                    let transfer_size = functional_transfer_size(alt.extra()).ok_or_else(|| {
                        Error::MissingTransferSize {
                            alt_setting: alt_setting.to_owned(),
                        }
                    })?;
                    handle
                        .claim_interface(alt.interface_number())
                        .map_err(Self::map_usb)?;
                    if let Err(error) =
                        handle.set_alternate_setting(alt.interface_number(), alt.setting_number())
                    {
                        let _ = handle.release_interface(alt.interface_number());
                        return Err(Self::map_usb(error));
                    }
                    self.session = Some(ClaimedSession {
                        handle,
                        interface: alt.interface_number(),
                        transfer_size,
                    });
                    return Ok(());
                }
            }
            return Err(Error::WrongAltSetting {
                expected: alt_setting.to_owned(),
                available: expected.alt_settings.clone(),
            });
        }
        Err(Error::DeviceNotFound(expected.path.clone()))
    }

    fn release(&mut self) -> Result<()> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        session
            .handle
            .release_interface(session.interface)
            .map_err(Self::map_usb)
    }

    fn transfer_size(&mut self) -> Result<usize> {
        Ok(self.session()?.transfer_size)
    }

    fn status(&mut self) -> Result<DfuStatus> {
        let session = self.session()?;
        let mut bytes = [0_u8; 6];
        let read = session
            .handle
            .read_control(
                rusb::request_type(
                    rusb::Direction::In,
                    rusb::RequestType::Class,
                    rusb::Recipient::Interface,
                ),
                REQUEST_GET_STATUS,
                0,
                session.interface.into(),
                &mut bytes,
                CONTROL_TIMEOUT,
            )
            .map_err(Self::map_usb)?;
        if read != bytes.len() {
            return Err(Error::MalformedStatus(read));
        }
        let poll_ms =
            u32::from(bytes[1]) | (u32::from(bytes[2]) << 8) | (u32::from(bytes[3]) << 16);
        Ok(DfuStatus {
            status: bytes[0],
            poll_timeout: Duration::from_millis(u64::from(poll_ms)),
            state: DfuState::from(bytes[4]),
        })
    }

    fn clear_status(&mut self) -> Result<()> {
        class_write(self.session()?, REQUEST_CLEAR_STATUS, 0, &[]).map(|_| ())
    }

    fn abort(&mut self) -> Result<()> {
        class_write(self.session()?, REQUEST_ABORT, 0, &[]).map(|_| ())
    }

    fn download_chunk(&mut self, block: u16, data: &[u8]) -> Result<usize> {
        class_write(self.session()?, REQUEST_DNLOAD, block, data)
    }

    fn finish_download(&mut self, block: u16) -> Result<()> {
        let written = class_write(self.session()?, REQUEST_DNLOAD, block, &[])?;
        if written != 0 {
            return Err(Error::ShortUsbWrite {
                expected: 0,
                actual: written,
            });
        }
        Ok(())
    }

    fn detach(&mut self, timeout: Duration) -> Result<()> {
        let millis = u16::try_from(timeout.as_millis()).unwrap_or(u16::MAX);
        class_write(self.session()?, REQUEST_DETACH, millis, &[]).map(|_| ())
    }

    fn reset(&mut self) -> Result<()> {
        self.session()?.handle.reset().map_err(Self::map_usb)
    }
}

fn class_write(session: &ClaimedSession, request: u8, value: u16, data: &[u8]) -> Result<usize> {
    session
        .handle
        .write_control(
            rusb::request_type(
                rusb::Direction::Out,
                rusb::RequestType::Class,
                rusb::Recipient::Interface,
            ),
            request,
            value,
            session.interface.into(),
            data,
            CONTROL_TIMEOUT,
        )
        .map_err(RusbTransport::map_usb)
}

fn functional_transfer_size(mut extra: &[u8]) -> Option<usize> {
    while extra.len() >= 2 {
        let length = usize::from(extra[0]);
        if length < 2 || length > extra.len() {
            return None;
        }
        if extra[1] == DFU_FUNCTIONAL_DESCRIPTOR && length >= 7 {
            let size = u16::from_le_bytes([extra[5], extra[6]]);
            return (size != 0).then_some(usize::from(size));
        }
        extra = &extra[length..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transfer_size_from_dfu_functional_descriptor() {
        let descriptor = [9, 0x21, 0x0b, 0xff, 0x00, 0x00, 0x10, 0x10, 0x01];
        assert_eq!(functional_transfer_size(&descriptor), Some(4096));
    }

    #[test]
    fn refuses_zero_or_malformed_transfer_size() {
        assert_eq!(functional_transfer_size(&[7, 0x21, 0, 0, 0, 0, 0]), None);
        assert_eq!(functional_transfer_size(&[9, 0x21, 0]), None);
    }
}
