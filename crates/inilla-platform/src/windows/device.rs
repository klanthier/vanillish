//! The device on Windows, for [`crate::device`]: the power request that keeps the screen and the
//! PC awake, the connection's cost (WinRT), free memory, the system's name (the registry), a web
//! page, a folder or a file shown in File Explorer, and the exe started over.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired,
    PowerSetRequest,
};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::SystemInformation::{
    GetWindowsDirectoryW, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// The power request while the screen is kept on. A request holds for the whole process, where
/// `SetThreadExecutionState` would hold only for the thread that asked (any of Bevy's).
static AWAKE: Mutex<Option<isize>> = Mutex::new(None);

/// Keep the display on and the PC from sleeping (`true`), or let both go (`false`). Idempotent.
pub(crate) fn keep_awake(on: bool) {
    let mut held = AWAKE.lock().unwrap_or_else(|e| e.into_inner());
    match (on, *held) {
        (true, None) => *held = request_awake(),
        (false, Some(h)) => {
            let h = h as HANDLE;
            // SAFETY: the request `request_awake` made, cleared and closed once.
            unsafe {
                PowerClearRequest(h, PowerRequestDisplayRequired);
                PowerClearRequest(h, PowerRequestSystemRequired);
                CloseHandle(h);
            }
            *held = None;
        }
        _ => {}
    }
}

/// A power request with the display and the system required, named for `powercfg /requests`.
fn request_awake() -> Option<isize> {
    /// `POWER_REQUEST_CONTEXT_VERSION`.
    const VERSION: u32 = 0;
    let mut reason = super::wide("Vanillish is running");
    let context = REASON_CONTEXT {
        Version: VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: reason.as_mut_ptr(),
        },
    };
    // SAFETY: `context` and its NUL-terminated string outlive the call, which copies them.
    let h = unsafe { PowerCreateRequest(&context) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: `h` is the request just made.
    unsafe {
        PowerSetRequest(h, PowerRequestDisplayRequired);
        PowerSetRequest(h, PowerRequestSystemRequired);
    }
    Some(h as isize)
}

/// The cost of the connection to the internet, or `None` with no connection or no answer.
/// windows-core joins this thread to the process's COM apartment if it has none.
pub(crate) fn network_metered() -> Option<bool> {
    use windows::Networking::Connectivity::NetworkInformation;
    let cost = NetworkInformation::GetInternetConnectionProfile()
        .ok()?
        .GetConnectionCost()
        .ok()?;
    crate::device::metered_from_windows(
        cost.NetworkCostType().ok()?.0,
        cost.Roaming().ok()?,
        cost.OverDataLimit().ok()?,
    )
}

/// The physical memory free now, system-wide (`ullAvailPhys`).
pub(crate) fn available_memory() -> Option<u64> {
    // SAFETY: plain integers, all valid zeroed.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is the size its `dwLength` says.
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullAvailPhys)
}

/// [`crate::device::describe_windows`] from the registry, read once (the settings screen asks
/// every frame). benilla's own `"windows x86_64"` when the registry does not answer.
pub(crate) fn describe() -> String {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        const CPU: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
        let Some(product) = reg_string(NT, "ProductName") else {
            return format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        };
        // `DisplayVersion` since 20H2; `ReleaseId` ("2004") before.
        let version = reg_string(NT, "DisplayVersion")
            .or_else(|| reg_string(NT, "ReleaseId"))
            .unwrap_or_default();
        crate::device::describe_windows(
            &product,
            &version,
            &reg_string(NT, "CurrentBuildNumber").unwrap_or_default(),
            reg_dword(NT, "UBR"),
            std::env::consts::ARCH,
            &reg_string(CPU, "ProcessorNameString").unwrap_or_default(),
        )
    })
    .clone()
}

/// A short `REG_SZ` under `HKEY_LOCAL_MACHINE\<key>`; `None` when missing, empty or long.
fn reg_string(key: &str, value: &str) -> Option<String> {
    let (key, value) = (super::wide(key), super::wide(value));
    let mut buf = [0u16; 256];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: both names are NUL-terminated; `buf` holds `size` bytes; the type is not asked.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..len]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// A `REG_DWORD` under `HKEY_LOCAL_MACHINE\<key>`.
fn reg_dword(key: &str, value: &str) -> Option<u32> {
    let (key, value) = (super::wide(key), super::wide(value));
    let mut v = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: as in `reg_string`, into a four-byte `v`.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut v as *mut u32).cast(),
            &mut size,
        )
    };
    (rc == ERROR_SUCCESS).then_some(v)
}

/// Open `url` in the default browser, off the caller's thread (the shell may take a moment, and
/// wants COM in a single-threaded apartment). Only a web address ([`crate::device::is_web_url`]).
pub(crate) fn open_url(url: &str) {
    if !crate::device::is_web_url(url) {
        return;
    }
    let url = super::wide(url);
    let _ = std::thread::Builder::new()
        .name("inilla-open-url".into())
        .spawn(move || {
            let verb = super::wide("open");
            // SAFETY: COM on this thread only, balanced below; both strings are NUL-terminated
            // and outlive the call.
            unsafe {
                let init = CoInitializeEx(
                    std::ptr::null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                );
                ShellExecuteW(
                    std::ptr::null_mut(),
                    verb.as_ptr(),
                    url.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                );
                if init >= 0 {
                    CoUninitialize();
                }
            }
        });
}

/// `explorer.exe` in the Windows folder, by its full path: a bare name is looked up in this exe's
/// own folder first.
fn explorer() -> PathBuf {
    let mut buf = [0u16; 260];
    // SAFETY: `buf` holds as many units as it says.
    let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return PathBuf::from("explorer.exe");
    }
    PathBuf::from(OsString::from_wide(&buf[..n])).join("explorer.exe")
}

/// File Explorer with `prefix` and `path` as its one argument, handed none of this process's
/// standard handles (the log capture's pipe). The path is quoted when it holds a space or a comma
/// (explorer splits its arguments at commas; a Windows name never holds a quote). It is
/// normalized first, so only a drive's root keeps a trailing separator — which before a closing
/// quote would read as an escaped one — and a root is never quoted.
fn explorer_with(prefix: &str, path: &Path) -> bool {
    let path = path.components().collect::<PathBuf>();
    let quote = path.to_string_lossy().contains([' ', ',']);
    let mut arg = OsString::from(prefix);
    if quote {
        arg.push("\"");
    }
    arg.push(path.as_os_str());
    if quote {
        arg.push("\"");
    }
    Command::new(explorer())
        .raw_arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// The folder at `path` in File Explorer.
pub(crate) fn open_folder(path: &Path) -> bool {
    path.is_dir() && explorer_with("", path)
}

/// The folder holding the file at `path` in File Explorer, the file selected.
pub(crate) fn show_in_explorer(path: &Path) -> bool {
    path.is_file() && explorer_with("/select,", path)
}

/// This exe again, with the same arguments. The new process's boot sets `WOW_DATA` and
/// `BENILLA_HOME` again from the same folder; its standard handles are not this process's (the
/// log capture's pipe, which ends with it).
pub(crate) fn relaunch() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env_remove("WOW_DATA")
        .env_remove("BENILLA_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}
