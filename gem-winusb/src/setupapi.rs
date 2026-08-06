use std::mem::size_of;

use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::{
        CM_Get_DevNode_Status, CM_PROB_FAILED_INSTALL, CR_SUCCESS, DIGCF_ALLCLASSES, DIGCF_PRESENT,
        SP_DEVINFO_DATA, SPDRP_COMPATIBLEIDS, SPDRP_DRIVER, SPDRP_HARDWAREID, SPDRP_SERVICE,
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDeviceRegistryPropertyW,
    },
    Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_DATA, ERROR_NO_MORE_ITEMS,
        ERROR_NOT_FOUND, GetLastError, INVALID_HANDLE_VALUE,
    },
};

use crate::{
    DriverState, T3_DFU_COMPATIBLE_ID, T3_DFU_HARDWARE_ID,
    model::{DeviceFacts, classify},
};

struct DeviceInfoSet(windows_sys::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: the handle came from `SetupDiGetClassDevsW` and is destroyed exactly once.
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

pub(crate) fn probe() -> DriverState {
    match probe_inner() {
        Ok(devices) => classify(&devices),
        Err(win32_error) => DriverState::ProbeFailed { win32_error },
    }
}

fn probe_inner() -> Result<Vec<DeviceFacts>, u32> {
    // SAFETY: null class/enumerator/parent with ALLCLASSES|PRESENT is the documented way to
    // enumerate every currently present devnode.
    let raw_set = unsafe {
        SetupDiGetClassDevsW(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        )
    };
    if raw_set == INVALID_HANDLE_VALUE as isize {
        // SAFETY: this immediately follows the failing Win32 call on the same thread.
        return Err(unsafe { GetLastError() });
    }
    let set = DeviceInfoSet(raw_set);
    let mut devices = Vec::new();
    let mut index = 0;

    loop {
        let mut data = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: `set` is valid and `data` has the required size for this architecture.
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) } == 0 {
            // SAFETY: this immediately follows the failing Win32 call on the same thread.
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_ITEMS {
                break;
            }
            return Err(error);
        }
        index += 1;

        let hardware_ids = registry_strings(set.0, &data, SPDRP_HARDWAREID)?;
        if !contains_id(&hardware_ids, T3_DFU_HARDWARE_ID) {
            continue;
        }

        let compatible_ids = registry_strings(set.0, &data, SPDRP_COMPATIBLEIDS)?;
        if !contains_id(&compatible_ids, T3_DFU_COMPATIBLE_ID) {
            continue;
        }

        let service = first_non_empty(registry_strings(set.0, &data, SPDRP_SERVICE)?);
        let driver_key = first_non_empty(registry_strings(set.0, &data, SPDRP_DRIVER)?);
        let mut status = 0;
        let mut problem_code = 0;
        // SAFETY: `data.DevInst` belongs to the present devnode and both outputs are live locals.
        let config_result =
            unsafe { CM_Get_DevNode_Status(&mut status, &mut problem_code, data.DevInst, 0) };
        if config_result != CR_SUCCESS {
            return Err(config_result);
        }

        // Code 28 is intentionally named here: the classifier remains platform-independent and
        // its literal is covered by tests, while this assertion catches a Windows binding drift.
        debug_assert_eq!(CM_PROB_FAILED_INSTALL, 28);
        devices.push(DeviceFacts {
            service,
            driver_key,
            problem_code,
        });
    }

    Ok(devices)
}

fn contains_id(values: &[String], expected: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(expected))
}

fn first_non_empty(values: Vec<String>) -> Option<String> {
    values.into_iter().find(|value| !value.is_empty())
}

fn registry_strings(
    set: windows_sys::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    data: &SP_DEVINFO_DATA,
    property: u32,
) -> Result<Vec<String>, u32> {
    let mut required = 0;
    let mut registry_type = 0;
    // SAFETY: the first call intentionally supplies no buffer to obtain the required byte count.
    let first = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            data,
            property,
            &mut registry_type,
            std::ptr::null_mut(),
            0,
            &mut required,
        )
    };
    if first == 0 {
        // SAFETY: this immediately follows the failing Win32 call on the same thread.
        let error = unsafe { GetLastError() };
        if matches!(
            error,
            ERROR_INVALID_DATA | ERROR_FILE_NOT_FOUND | ERROR_NOT_FOUND
        ) {
            return Ok(Vec::new());
        }
        if error != ERROR_INSUFFICIENT_BUFFER {
            return Err(error);
        }
    }
    if required == 0 {
        return Ok(Vec::new());
    }

    let mut raw = vec![0_u8; required as usize];
    // SAFETY: `raw` has exactly the requested byte capacity and remains live for the call.
    if unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            data,
            property,
            &mut registry_type,
            raw.as_mut_ptr(),
            raw.len() as u32,
            &mut required,
        )
    } == 0
    {
        // SAFETY: this immediately follows the failing Win32 call on the same thread.
        return Err(unsafe { GetLastError() });
    }

    raw.truncate(required as usize);
    if !raw.len().is_multiple_of(2) {
        return Err(ERROR_INVALID_DATA);
    }
    let utf16: Vec<u16> = raw
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();

    Ok(utf16
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect())
}
