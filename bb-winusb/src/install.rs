use std::{
    ffi::{CStr, CString, OsString, c_char, c_void},
    fs::{File, OpenOptions},
    io::Read,
    mem::{size_of, transmute_copy},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    ptr::null_mut,
    time::Duration,
};

use rusb::UsbContext as _;
use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_CANCELLED, FreeLibrary, GetLastError, HMODULE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0,
    },
    Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    Storage::FileSystem::FILE_SHARE_READ,
    System::{
        Com::CoTaskMemFree,
        LibraryLoader::{
            GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
            LoadLibraryExW, SetDefaultDllDirectories,
        },
        Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
    },
    UI::{
        Shell::{
            FOLDERID_Windows, KF_FLAG_CREATE, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
            SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHGetKnownFolderPath, ShellExecuteExW,
        },
        WindowsAndMessaging::SW_HIDE,
    },
};

use crate::{
    DriverState, T3_DFU_COMPATIBLE_ID, T3_DFU_DEVICE_INTERFACE_GUID, T3_DFU_HARDWARE_ID, probe,
};

const LIBWDI_DLL_NAME: &str = "libwdi.dll";
const LIBWDI_DLL_SHA256: [u8; 32] = [
    0xc9, 0xf0, 0xaa, 0xa5, 0xa1, 0xb0, 0xa7, 0x1b, 0x17, 0x40, 0x25, 0x61, 0x68, 0xe3, 0xf0, 0xa8,
    0x70, 0xe9, 0x79, 0x14, 0x97, 0x65, 0xf0, 0xe2, 0x77, 0x8b, 0x16, 0x03, 0x77, 0xb6, 0x9f, 0x27,
];

const HELPER_NAME: &str = "bb-winusb-helper.exe";
const HELPER_COMMAND: &str = "install-t3-rom-dfu";

pub const HELPER_EXIT_INSTALLED: i32 = 0;
pub const HELPER_EXIT_ALREADY_READY: i32 = 11;
pub const HELPER_EXIT_NO_DEVICE: i32 = 12;
pub const HELPER_EXIT_MULTIPLE: i32 = 13;
pub const HELPER_EXIT_CONFLICT: i32 = 14;
pub const HELPER_EXIT_RUNTIME: i32 = 20;
pub const HELPER_EXIT_PREPARE: i32 = 21;
pub const HELPER_EXIT_INSTALL: i32 = 22;
pub const HELPER_EXIT_VERIFY: i32 = 23;
pub const HELPER_EXIT_INVALID_COMMAND: i32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed,
    AlreadyReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    NoDevice,
    MultipleCandidates,
    DriverConflict(DriverState),
    ElevationCancelled,
    HelperNotFound,
    HelperIntegrityUnavailable,
    HelperIntegrityMismatch,
    HelperFailed(u32),
    Runtime(String),
    Prepare { code: i32, detail: String },
    Install { code: i32, detail: String },
    PostInstall(DriverState),
    TransportVerification(String),
}

impl InstallError {
    /// Stable, non-secret process status used by the consoleless helper.
    pub const fn helper_exit_code(&self) -> i32 {
        match self {
            Self::NoDevice => HELPER_EXIT_NO_DEVICE,
            Self::MultipleCandidates => HELPER_EXIT_MULTIPLE,
            Self::DriverConflict(_) => HELPER_EXIT_CONFLICT,
            Self::Prepare { .. } => HELPER_EXIT_PREPARE,
            Self::Install { .. } => HELPER_EXIT_INSTALL,
            Self::PostInstall(_) | Self::TransportVerification(_) => HELPER_EXIT_VERIFY,
            Self::ElevationCancelled
            | Self::HelperNotFound
            | Self::HelperIntegrityUnavailable
            | Self::HelperIntegrityMismatch
            | Self::HelperFailed(_)
            | Self::Runtime(_) => HELPER_EXIT_RUNTIME,
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => f.write_str("the exact T3 ROM DFU device is no longer present"),
            Self::MultipleCandidates => f.write_str("more than one T3 ROM DFU device is present"),
            Self::DriverConflict(state) => {
                write!(
                    f,
                    "the device is not in the safe driverless state: {state:?}"
                )
            }
            Self::ElevationCancelled => f.write_str("administrator approval was cancelled"),
            Self::HelperNotFound => f.write_str("the WinUSB installer helper is missing"),
            Self::HelperIntegrityUnavailable => {
                f.write_str("this build has no pinned WinUSB helper identity")
            }
            Self::HelperIntegrityMismatch => {
                f.write_str("the WinUSB installer helper failed its integrity check")
            }
            Self::HelperFailed(code) => write!(f, "the installer helper failed (exit {code})"),
            Self::Runtime(detail) => write!(f, "WinUSB runtime error: {detail}"),
            Self::Prepare { code, detail } => {
                write!(f, "libwdi could not prepare the driver ({code}: {detail})")
            }
            Self::Install { code, detail } => {
                write!(f, "libwdi could not install the driver ({code}: {detail})")
            }
            Self::PostInstall(state) => {
                write!(
                    f,
                    "WinUSB was installed but readiness was not confirmed: {state:?}"
                )
            }
            Self::TransportVerification(detail) => {
                write!(
                    f,
                    "WinUSB is active but libusb could not open the DFU device: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for InstallError {}

/// Ask Windows to launch the fixed helper with `runas`, wait without opening a console, and map
/// its fixed exit protocol. No device ID, path, INF name or driver choice crosses this boundary.
pub fn launch_elevated_helper(
    expected_helper_sha256: &str,
) -> Result<InstallOutcome, InstallError> {
    let current = std::env::current_exe()
        .map_err(|error| InstallError::Runtime(format!("current executable: {error}")))?;
    let helper = current
        .parent()
        .ok_or_else(|| InstallError::Runtime("executable has no parent directory".into()))?
        .join(HELPER_NAME);
    if !helper.is_file() {
        return Err(InstallError::HelperNotFound);
    }
    let helper = helper
        .canonicalize()
        .map_err(|error| InstallError::Runtime(format!("canonicalize helper: {error}")))?;
    // Keep a read-only sharing handle open across ShellExecuteEx. This closes the hash-to-exec
    // replacement window while still allowing the Windows loader to read the executable.
    let helper_guard = verify_helper(&helper, expected_helper_sha256)?;

    let verb = wide("runas");
    let file = wide_path(&helper);
    let parameters = wide(HELPER_COMMAND);
    let directory = wide_path(helper.parent().expect("helper path has a parent"));
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        hwnd: null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_HIDE,
        ..Default::default()
    };

    // SAFETY: all UTF-16 buffers outlive the call and `execute` has the correct architecture size.
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        // SAFETY: this immediately follows the failing Win32 call on the same thread.
        let error = unsafe { GetLastError() };
        return if error == ERROR_CANCELLED {
            Err(InstallError::ElevationCancelled)
        } else {
            Err(InstallError::Runtime(format!(
                "ShellExecuteExW failed with {error}"
            )))
        };
    }
    drop(helper_guard);
    if execute.hProcess.is_null() || execute.hProcess == INVALID_HANDLE_VALUE {
        return Err(InstallError::Runtime(
            "ShellExecuteExW returned no process handle".into(),
        ));
    }
    let process = ProcessHandle(execute.hProcess);

    // SAFETY: `process` is a live process handle returned with SEE_MASK_NOCLOSEPROCESS.
    let wait = unsafe { WaitForSingleObject(process.0, INFINITE) };
    if wait != WAIT_OBJECT_0 {
        return Err(InstallError::Runtime(format!(
            "waiting for the helper failed with wait status {wait}"
        )));
    }
    let mut exit_code = 0;
    // SAFETY: `process` is still live and `exit_code` is a writable local.
    if unsafe { GetExitCodeProcess(process.0, &mut exit_code) } == 0 {
        // SAFETY: this immediately follows the failing Win32 call on the same thread.
        return Err(InstallError::Runtime(format!(
            "GetExitCodeProcess failed with {}",
            unsafe { GetLastError() }
        )));
    }

    match exit_code as i32 {
        HELPER_EXIT_INSTALLED => Ok(InstallOutcome::Installed),
        HELPER_EXIT_ALREADY_READY => Ok(InstallOutcome::AlreadyReady),
        HELPER_EXIT_NO_DEVICE => Err(InstallError::NoDevice),
        HELPER_EXIT_MULTIPLE => Err(InstallError::MultipleCandidates),
        HELPER_EXIT_CONFLICT => Err(InstallError::DriverConflict(probe())),
        HELPER_EXIT_RUNTIME | HELPER_EXIT_PREPARE | HELPER_EXIT_INSTALL | HELPER_EXIT_VERIFY => {
            Err(InstallError::HelperFailed(exit_code))
        }
        _ => Err(InstallError::HelperFailed(exit_code)),
    }
}

struct ProcessHandle(windows_sys::Win32::Foundation::HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this handle is owned by this RAII value and is closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Elevated helper entry point. It is intentionally argument-free at the API level and repeats
/// the exact SetupAPI policy before libwdi is loaded.
pub fn install_t3_rom_dfu() -> Result<InstallOutcome, InstallError> {
    match probe() {
        DriverState::NeedsInstall => {}
        DriverState::ReadyWinUsb | DriverState::ReadyExternal { .. } => {
            return Ok(InstallOutcome::AlreadyReady);
        }
        DriverState::NoDevice => return Err(InstallError::NoDevice),
        DriverState::MultipleCandidates { .. } => return Err(InstallError::MultipleCandidates),
        state => return Err(InstallError::DriverConflict(state)),
    }

    let runtime_path = locate_runtime()?;
    verify_runtime(&runtime_path)?;
    let libwdi = LibWdi::load(&runtime_path)?;
    let mut list = libwdi.create_list()?;
    let selected = list.single_driverless_t3()?;

    let staging = StagingDirectory::create()?;
    let staging_utf8 = CString::new(staging.path().to_string_lossy().as_bytes())
        .map_err(|_| InstallError::Runtime("driver staging path contains a NUL".into()))?;
    let inf_name = CString::new("t3gemstone_am62x_rom_dfu_winusb.inf").expect("static string");
    let vendor = CString::new("T3 Gemstone").expect("static string");
    let guid = CString::new(T3_DFU_DEVICE_INTERFACE_GUID).expect("static string");
    let cert = CString::new("CN=T3 Gemstone AM62x ROM DFU MVP").expect("static string");
    let fallback_description = CString::new("T3 Gemstone AM62x ROM DFU").expect("static string");

    // Copy the list record so a fallback description can be supplied without making
    // `wdi_destroy_list` free memory owned by Rust.
    let mut device = unsafe { *selected };
    device.next = null_mut();
    if device.desc.is_null() {
        device.desc = fallback_description.as_ptr().cast_mut();
    }
    let mut prepare_options = WdiOptionsPrepareDriver {
        driver_type: WDI_WINUSB,
        vendor_name: vendor.as_ptr().cast_mut(),
        device_guid: guid.as_ptr().cast_mut(),
        disable_cat: 0,
        disable_signing: 0,
        cert_subject: cert.as_ptr().cast_mut(),
        use_wcid_driver: 0,
        external_inf: 0,
    };
    // SAFETY: all C strings and the copied device record remain live for this synchronous call.
    let prepared = unsafe {
        (libwdi.prepare_driver)(
            &mut device,
            staging_utf8.as_ptr(),
            inf_name.as_ptr(),
            &mut prepare_options,
        )
    };
    if prepared != WDI_SUCCESS {
        return Err(InstallError::Prepare {
            code: prepared,
            detail: libwdi.error_text(prepared),
        });
    }

    let mut install_options = WdiOptionsInstallDriver {
        hwnd: null_mut(),
        install_filter_driver: 0,
        pending_install_timeout: 30_000,
    };
    // SAFETY: the prepared files and all FFI values remain live for this synchronous call.
    let installed = unsafe {
        (libwdi.install_driver)(
            &mut device,
            staging_utf8.as_ptr(),
            inf_name.as_ptr(),
            &mut install_options,
        )
    };
    if installed != WDI_SUCCESS {
        return Err(InstallError::Install {
            code: installed,
            detail: libwdi.error_text(installed),
        });
    }

    drop(list);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut last_transport_error = None;
    loop {
        let state = probe();
        if matches!(state, DriverState::ReadyWinUsb) {
            match verify_dfu_transport() {
                Ok(()) => return Ok(InstallOutcome::Installed),
                Err(error) => last_transport_error = Some(error),
            }
        }
        if std::time::Instant::now() >= deadline {
            if let Some(error) = last_transport_error {
                return Err(InstallError::TransportVerification(error));
            }
            return Err(InstallError::PostInstall(state));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Exercise the same libusb open boundary used by the DFU flasher. SetupAPI reporting WinUSB is
/// necessary but not sufficient: this catches an unusable backend before the GUI says “ready”.
pub fn verify_dfu_transport() -> Result<(), String> {
    let context = rusb::Context::new().map_err(|error| error.to_string())?;
    let devices = context.devices().map_err(|error| error.to_string())?;

    for device in devices.iter() {
        let descriptor = match device.device_descriptor() {
            Ok(descriptor) => descriptor,
            Err(_) => continue,
        };
        if descriptor.vendor_id() != 0x0451
            || descriptor.product_id() != 0x6165
            || descriptor.device_version() != rusb::Version::from_bcd(0x0200)
        {
            continue;
        }
        let has_exact_dfu_interface = (0..descriptor.num_configurations()).any(|index| {
            device.config_descriptor(index).is_ok_and(|config| {
                config.interfaces().any(|interface| {
                    interface.descriptors().any(|alt| {
                        alt.class_code() == 0xfe
                            && alt.sub_class_code() == 0x01
                            && alt.protocol_code() == 0x02
                    })
                })
            })
        });
        if !has_exact_dfu_interface {
            continue;
        }
        return device.open().map(drop).map_err(|error| error.to_string());
    }

    Err("the exact ROM DFU USB descriptors were not found".into())
}

struct StagingDirectory(PathBuf);

impl StagingDirectory {
    fn create() -> Result<Self, InstallError> {
        // The Windows directory is ACL-protected. A cryptographically unpredictable direct child
        // of its Temp directory avoids trusting a user-creatable ProgramData parent.
        let root = windows_directory()?.join("Temp");
        let mut random = [0_u8; 16];
        // SAFETY: `random` is a writable buffer and the system-preferred provider takes no handle.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                random.as_mut_ptr(),
                random.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(InstallError::Runtime(format!(
                "BCryptGenRandom failed with status {status:#x}"
            )));
        }
        let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = root.join(format!("T3Gemstone-WinUSB-{nonce}"));
        std::fs::create_dir(&path).map_err(|error| {
            InstallError::Runtime(format!("create protected staging transaction: {error}"))
        })?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        // This guard owns the exact random directory it created. Driver Store staging is complete
        // before libwdi returns, so its temporary INF/CAT/co-installer files are no longer needed.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn windows_directory() -> Result<PathBuf, InstallError> {
    let mut raw = null_mut();
    // SAFETY: `raw` is a live output pointer. A null token requests the current machine path.
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_Windows,
            KF_FLAG_CREATE as u32,
            null_mut(),
            &mut raw,
        )
    };
    if result < 0 || raw.is_null() {
        return Err(InstallError::Runtime(format!(
            "SHGetKnownFolderPath(Windows) failed with {result:#x}"
        )));
    }
    let mut len = 0;
    // SAFETY: the API returns a NUL-terminated CoTaskMem string on success.
    unsafe {
        while *raw.add(len) != 0 {
            len += 1;
        }
    }
    // SAFETY: `raw..raw+len` is the UTF-16 string measured above.
    let path = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw, len)
    }));
    // SAFETY: SHGetKnownFolderPath allocates this pointer with CoTaskMemAlloc.
    unsafe {
        CoTaskMemFree(raw.cast());
    }
    Ok(path)
}

fn locate_runtime() -> Result<PathBuf, InstallError> {
    let current = std::env::current_exe()
        .map_err(|error| InstallError::Runtime(format!("current executable: {error}")))?;
    let sibling = current
        .parent()
        .ok_or_else(|| InstallError::Runtime("helper executable has no parent".into()))?
        .join(LIBWDI_DLL_NAME);
    if sibling.is_file() {
        return sibling
            .canonicalize()
            .map_err(|error| InstallError::Runtime(format!("canonicalize libwdi: {error}")));
    }

    #[cfg(debug_assertions)]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("native")
            .join("x86_64-pc-windows-msvc")
            .join(LIBWDI_DLL_NAME);
        if development.is_file() {
            return development.canonicalize().map_err(|error| {
                InstallError::Runtime(format!("canonicalize development libwdi: {error}"))
            });
        }
    }

    Err(InstallError::Runtime(format!(
        "{LIBWDI_DLL_NAME} is missing beside the helper"
    )))
}

fn verify_runtime(path: &Path) -> Result<(), InstallError> {
    let mut file = File::open(path)
        .map_err(|error| InstallError::Runtime(format!("open libwdi runtime: {error}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| InstallError::Runtime(format!("hash libwdi runtime: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hasher.finalize();
    if actual[..] != LIBWDI_DLL_SHA256 {
        return Err(InstallError::Runtime(
            "libwdi runtime SHA-256 does not match the pinned build".into(),
        ));
    }
    Ok(())
}

fn verify_helper(path: &Path, expected_hex: &str) -> Result<File, InstallError> {
    let expected = decode_sha256(expected_hex).ok_or(InstallError::HelperIntegrityUnavailable)?;
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|error| InstallError::Runtime(format!("open WinUSB helper: {error}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| InstallError::Runtime(format!("hash WinUSB helper: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize()[..] != expected {
        return Err(InstallError::HelperIntegrityMismatch);
    }
    Ok(file)
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, output) in decoded.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(decoded)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WdiDeviceInfo {
    next: *mut WdiDeviceInfo,
    vid: u16,
    pid: u16,
    is_composite: i32,
    mi: u8,
    desc: *mut c_char,
    driver: *mut c_char,
    device_id: *mut c_char,
    hardware_id: *mut c_char,
    compatible_id: *mut c_char,
    upper_filter: *mut c_char,
    driver_version: u64,
}

#[repr(C)]
struct WdiOptionsCreateList {
    list_all: i32,
    list_hubs: i32,
    trim_whitespaces: i32,
}

#[repr(C)]
struct WdiOptionsPrepareDriver {
    driver_type: i32,
    vendor_name: *mut c_char,
    device_guid: *mut c_char,
    disable_cat: i32,
    disable_signing: i32,
    cert_subject: *mut c_char,
    use_wcid_driver: i32,
    external_inf: i32,
}

#[repr(C)]
struct WdiOptionsInstallDriver {
    hwnd: windows_sys::Win32::Foundation::HWND,
    install_filter_driver: i32,
    pending_install_timeout: u32,
}

const WDI_SUCCESS: i32 = 0;
const WDI_WINUSB: i32 = 0;
const WDI_LOG_LEVEL_NONE: i32 = 4;

type WdiCreateList =
    unsafe extern "system" fn(*mut *mut WdiDeviceInfo, *mut WdiOptionsCreateList) -> i32;
type WdiDestroyList = unsafe extern "system" fn(*mut WdiDeviceInfo) -> i32;
type WdiPrepareDriver = unsafe extern "system" fn(
    *mut WdiDeviceInfo,
    *const c_char,
    *const c_char,
    *mut WdiOptionsPrepareDriver,
) -> i32;
type WdiInstallDriver = unsafe extern "system" fn(
    *mut WdiDeviceInfo,
    *const c_char,
    *const c_char,
    *mut WdiOptionsInstallDriver,
) -> i32;
type WdiStrError = unsafe extern "system" fn(i32) -> *const c_char;
type WdiSetLogLevel = unsafe extern "system" fn(i32) -> i32;

struct LibWdi {
    module: HMODULE,
    create_list: WdiCreateList,
    destroy_list: WdiDestroyList,
    prepare_driver: WdiPrepareDriver,
    install_driver: WdiInstallDriver,
    strerror: WdiStrError,
}

impl LibWdi {
    fn load(path: &Path) -> Result<Self, InstallError> {
        // This process is the dedicated helper, so tightening its process-wide DLL policy cannot
        // disrupt GUI plugins or unrelated libraries.
        // SAFETY: the flags are the documented safe-search subset.
        if unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) } == 0 {
            // SAFETY: this immediately follows the failing Win32 call on the same thread.
            return Err(InstallError::Runtime(format!(
                "SetDefaultDllDirectories failed with {}",
                unsafe { GetLastError() }
            )));
        }
        let wide = wide_path(path);
        // SAFETY: `wide` is an absolute, NUL-terminated path whose SHA-256 was verified above.
        let module = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            // SAFETY: this immediately follows the failing Win32 call on the same thread.
            return Err(InstallError::Runtime(format!(
                "LoadLibraryExW(libwdi) failed with {}",
                unsafe { GetLastError() }
            )));
        }

        // SAFETY: every requested name and signature is copied from libwdi 1.5.1's public header.
        let result = unsafe {
            Ok(Self {
                module,
                create_list: symbol(module, b"wdi_create_list\0")?,
                destroy_list: symbol(module, b"wdi_destroy_list\0")?,
                prepare_driver: symbol(module, b"wdi_prepare_driver\0")?,
                install_driver: symbol(module, b"wdi_install_driver\0")?,
                strerror: symbol(module, b"wdi_strerror\0")?,
            })
        };
        if result.is_err() {
            // SAFETY: no function pointer is used after this failure path.
            unsafe {
                FreeLibrary(module);
            }
        }
        let library = result?;
        // SAFETY: this is a side-effect-free logging preference call on the loaded library.
        unsafe {
            let set_log: WdiSetLogLevel = symbol(library.module, b"wdi_set_log_level\0")?;
            set_log(WDI_LOG_LEVEL_NONE);
        }
        Ok(library)
    }

    fn create_list(&self) -> Result<DeviceList, InstallError> {
        let mut head = null_mut();
        let mut options = WdiOptionsCreateList {
            list_all: 1,
            list_hubs: 0,
            trim_whitespaces: 1,
        };
        // SAFETY: both output/list options pointers are valid for this synchronous call.
        let result = unsafe { (self.create_list)(&mut head, &mut options) };
        if result != WDI_SUCCESS {
            return Err(InstallError::Runtime(format!(
                "wdi_create_list failed ({result}: {})",
                self.error_text(result)
            )));
        }
        Ok(DeviceList {
            head,
            destroy: self.destroy_list,
        })
    }

    fn error_text(&self, code: i32) -> String {
        // SAFETY: libwdi returns a static NUL-terminated error string for every integer code.
        let raw = unsafe { (self.strerror)(code) };
        if raw.is_null() {
            return "unknown libwdi error".into();
        }
        // SAFETY: checked non-null and documented as a NUL-terminated static string.
        unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for LibWdi {
    fn drop(&mut self) {
        // SAFETY: this module is owned and all list guards are dropped before the library local.
        unsafe {
            FreeLibrary(self.module);
        }
    }
}

struct DeviceList {
    head: *mut WdiDeviceInfo,
    destroy: WdiDestroyList,
}

impl DeviceList {
    fn single_driverless_t3(&mut self) -> Result<*mut WdiDeviceInfo, InstallError> {
        let mut cursor = self.head;
        let mut matches = Vec::new();
        let mut visited = 0;
        while !cursor.is_null() {
            visited += 1;
            if visited > 4096 {
                return Err(InstallError::Runtime(
                    "libwdi returned an invalid cyclic/oversized device list".into(),
                ));
            }
            // SAFETY: cursor belongs to the live list returned by libwdi.
            let device = unsafe { &*cursor };
            if device.vid == 0x0451
                && device.pid == 0x6165
                && device.is_composite == 0
                && c_string_is_empty(device.driver)
                && c_string_eq_ignore_ascii_case(device.hardware_id, T3_DFU_HARDWARE_ID)
                && c_string_eq_ignore_ascii_case(device.compatible_id, T3_DFU_COMPATIBLE_ID)
            {
                matches.push(cursor);
            }
            cursor = device.next;
        }
        match matches.as_slice() {
            [only] => Ok(*only),
            [] => Err(InstallError::NoDevice),
            _ => Err(InstallError::MultipleCandidates),
        }
    }
}

impl Drop for DeviceList {
    fn drop(&mut self) {
        if !self.head.is_null() {
            // SAFETY: `head` is owned by this guard and destroyed exactly once before DLL unload.
            unsafe {
                (self.destroy)(self.head);
            }
        }
    }
}

fn c_string_is_empty(value: *const c_char) -> bool {
    value.is_null()
        // SAFETY: a non-null field in a live libwdi device record is a NUL-terminated C string.
        || unsafe { CStr::from_ptr(value) }.to_bytes().is_empty()
}

fn c_string_eq_ignore_ascii_case(value: *const c_char, expected: &str) -> bool {
    if value.is_null() {
        return false;
    }
    // SAFETY: a non-null field in a live libwdi device record is a NUL-terminated C string.
    unsafe { CStr::from_ptr(value) }
        .to_bytes()
        .eq_ignore_ascii_case(expected.as_bytes())
}

unsafe fn symbol<T: Copy>(module: HMODULE, name: &'static [u8]) -> Result<T, InstallError> {
    // SAFETY: caller guarantees `name` is NUL-terminated and `module` is loaded.
    let procedure = unsafe { GetProcAddress(module, name.as_ptr()) }.ok_or_else(|| {
        InstallError::Runtime(format!(
            "libwdi export {} is missing",
            String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
        ))
    })?;
    let address = procedure as *const () as *const c_void;
    debug_assert_eq!(size_of::<T>(), size_of::<*const c_void>());
    // SAFETY: the symbol's public C signature is encoded by `T` at each call site.
    Ok(unsafe { transmute_copy(&address) })
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_layout_matches_the_x64_libwdi_header() {
        assert_eq!(size_of::<WdiDeviceInfo>(), 80);
        assert_eq!(size_of::<WdiOptionsCreateList>(), 12);
        assert_eq!(size_of::<WdiOptionsPrepareDriver>(), 48);
        assert_eq!(size_of::<WdiOptionsInstallDriver>(), 16);
    }

    #[test]
    fn pinned_runtime_hash_is_sha256_sized() {
        assert_eq!(LIBWDI_DLL_SHA256.len(), 32);
    }

    #[test]
    fn helper_sha256_parser_is_strict() {
        assert_eq!(decode_sha256(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(decode_sha256("ab"), None);
        assert_eq!(decode_sha256(&"zz".repeat(32)), None);
    }
}
