//! logcat, through `liblog` — where an Android app's stdout and stderr would otherwise go nowhere
//! (fds 1 and 2 are `/dev/null` in an app process).

use std::ffi::{c_char, c_int, CString};

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const TAG: &[u8] = b"vanillish\0";
/// logcat drops a payload above ~4068 bytes; split well below.
const CHUNK: usize = 4000;

/// Write one captured line to logcat at the priority its text suggests.
pub(crate) fn write_line(line: &str) {
    let line = line.trim_end_matches(['\n', '\r']);
    if line.is_empty() {
        return;
    }
    let prio = crate::stdio::logcat_priority(line);
    let mut rest = line;
    while !rest.is_empty() {
        let mut cut = rest.len().min(CHUNK);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let (head, tail) = rest.split_at(cut);
        rest = tail;
        let Ok(text) = CString::new(head.replace('\0', "\u{FFFD}")) else {
            continue;
        };
        // SAFETY: both pointers are NUL-terminated and live for the call.
        unsafe {
            __android_log_write(prio, TAG.as_ptr().cast(), text.as_ptr());
        }
    }
}
