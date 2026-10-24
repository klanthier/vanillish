//! `SetStdHandle` for [`crate::stdio::capture`]. Rust's `std` looks the standard handle up on
//! every write, so pointing standard output and error at a pipe moves `println!`, tracing's
//! stderr writer and a panic's message into the reader at once.
//!
//! The pass-through goes where the handles pointed before: a console (`--console`, or the dev
//! build's own) in UTF-16, a file or pipe the parent redirected into as bytes, and with neither
//! (a GUI exe started from Explorer) the debugger's output, which DebugView shows. The C runtime's
//! descriptors 1 and 2 are left alone: the exe's C code (Lua, whose `print` the UI removes)
//! writes nothing through them.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::mem::ManuallyDrop;
use std::os::windows::io::{FromRawHandle, IntoRawHandle, OwnedHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_CHAR};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetStdHandle, WriteConsoleW, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;

/// Where each captured line also goes.
enum Pass {
    /// A console: its text is UTF-16.
    Console(isize),
    /// A file or a pipe: the bytes as they came. Never closed: it is the process's own handle.
    File(ManuallyDrop<File>),
    /// No handle at all, or `NUL`: `OutputDebugStringW`.
    Debugger,
}

impl Pass {
    fn of(handle: isize) -> Pass {
        if handle == 0 || handle == INVALID_HANDLE_VALUE as isize {
            return Pass::Debugger;
        }
        let h = handle as HANDLE;
        let mut mode = 0;
        // SAFETY: both only query a handle this process holds.
        if unsafe { GetConsoleMode(h, &mut mode) } != 0 {
            return Pass::Console(handle);
        }
        // A character device that is no console is `NUL` (what a relaunch hands its child):
        // nobody reads it.
        if unsafe { GetFileType(h) } == FILE_TYPE_CHAR {
            return Pass::Debugger;
        }
        // SAFETY: nothing closes a standard handle, so it stays open for the life of the
        // process, and `ManuallyDrop` keeps this `File` from closing it either.
        Pass::File(ManuallyDrop::new(unsafe { File::from_raw_handle(h) }))
    }

    fn write(&mut self, raw: &[u8], line: &str) {
        match self {
            Pass::Console(h) => write_console(*h as HANDLE, &String::from_utf8_lossy(raw)),
            Pass::File(f) => {
                let _ = f.write_all(raw);
            }
            Pass::Debugger => {
                let text = super::wide(line);
                // SAFETY: `text` is NUL-terminated and outlives the call.
                unsafe { OutputDebugStringW(text.as_ptr()) };
            }
        }
    }
}

/// `text` into the console at `h`, a bounded piece at a time, never splitting a surrogate pair.
fn write_console(h: HANDLE, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut rest = &wide[..];
    while !rest.is_empty() {
        let mut n = rest.len().min(4096);
        if n < rest.len() && (0xD800..0xDC00).contains(&rest[n - 1]) {
            n -= 1;
        }
        let mut written = 0u32;
        // SAFETY: `rest` holds at least `n` units; the console handle is the process's own.
        let ok =
            unsafe { WriteConsoleW(h, rest.as_ptr(), n as u32, &mut written, std::ptr::null()) };
        if ok == 0 || written == 0 {
            return;
        }
        rest = &rest[(written as usize).min(rest.len())..];
    }
}

/// Points standard output and error back at what they were when the reader ends or unwinds,
/// before its read end closes: a write into a pipe nobody drains fails (`ERROR_NO_DATA`), which
/// `eprint!` turns into a panic in whichever thread logs next.
struct Restore {
    out: isize,
    err: isize,
}

impl Drop for Restore {
    fn drop(&mut self) {
        // SAFETY: the original handles, which nothing closed (see `Pass::of`).
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, self.out as HANDLE);
            SetStdHandle(STD_ERROR_HANDLE, self.err as HANDLE);
        }
    }
}

/// Start the reader and point both standard handles at its pipe. `Err` with nothing changed.
pub(crate) fn capture(log_file: Option<&Path>) -> Result<(), String> {
    let mut file = match log_file {
        Some(p) => {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            OpenOptions::new().create(true).append(true).open(p).ok()
        }
        None => None,
    };
    // SAFETY: reads the process's standard handles.
    let (out, err) = unsafe {
        (
            GetStdHandle(STD_OUTPUT_HANDLE) as isize,
            GetStdHandle(STD_ERROR_HANDLE) as isize,
        )
    };
    // A redirect first (`> out.txt` leaves standard error at the console), standard error's
    // before standard output's as the unix arm passes through; then a console; then the debugger.
    let mut pass = match (Pass::of(err), Pass::of(out)) {
        (e @ Pass::File(_), _) => e,
        (_, o @ Pass::File(_)) => o,
        (e @ Pass::Console(_), _) => e,
        (_, o) => o,
    };
    let (reader, writer) = std::io::pipe().map_err(|e| format!("pipe: {e}"))?;
    let restore = Restore { out, err };
    std::thread::Builder::new()
        .name("inilla-stdio".into())
        .spawn(move || {
            let mut r = BufReader::new(reader);
            // Declared after `r`, so dropped (and run) before `r` closes the pipe.
            let _restore = restore;
            let mut raw = Vec::with_capacity(512);
            loop {
                raw.clear();
                match r.read_until(b'\n', &mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let line = crate::stdio::strip_ansi(&String::from_utf8_lossy(&raw));
                pass.write(&raw, &line);
                inilla_core::logring::global().push(&line);
                if let Some(f) = file.as_mut() {
                    let _ = f.write_all(line.as_bytes());
                    if !line.ends_with('\n') {
                        let _ = f.write_all(b"\n");
                    }
                    let _ = f.flush();
                }
            }
        })
        .map_err(|e| e.to_string())?;
    // Kept open for the life of the process: `std` looks the handle up on every write.
    let w = writer.into_raw_handle();
    // SAFETY: `w` is the pipe's write end, open from here on.
    let set = unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, w) != 0 && SetStdHandle(STD_ERROR_HANDLE, w) != 0
    };
    if !set {
        // SAFETY: the originals, as `Restore` puts them back; then the write end closes, the
        // reader reads its end and stops.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, out as HANDLE);
            SetStdHandle(STD_ERROR_HANDLE, err as HANDLE);
            drop(OwnedHandle::from_raw_handle(w));
        }
        return Err("SetStdHandle failed".into());
    }
    Ok(())
}
