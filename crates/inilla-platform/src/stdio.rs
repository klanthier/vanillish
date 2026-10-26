//! **Capture stdout and stderr** into the log ring (for the remote console and the on-device log
//! viewer) and, optionally, a session log file — without losing them where they went before.
//!
//! benilla prints a great deal of its diagnostics with `println!`/`eprintln!` (the build banner,
//! the preflight, every subsystem's one-line status), and on an iPhone nobody is attached to those
//! file descriptors. So both are pointed at a pipe; a reader thread splits the stream into lines,
//! strips terminal colour codes, pushes each line into [`inilla_core::logring::global`], appends
//! it to the log file (flushed per line, so a crash leaves everything up to its last words) and
//! passes the original bytes through to the old stderr (Xcode's console, `idevicesyslog`, a
//! terminal on desktop).
//!
//! Vanillish: an Android app's stdout and stderr are `/dev/null`, so there the pass-through goes
//! to **logcat** instead — except tracing's own lines, which bevy_log already sends to logcat
//! itself (they stay in the ring and the session log). On Windows the standard handles are
//! pointed at the pipe (`SetStdHandle`, `crate::windows::stdio`): a GUI exe has no console, so
//! the capture is the only log there is.

#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static CAPTURED: AtomicBool = AtomicBool::new(false);

/// Start capturing. Idempotent; returns an error string when the pipe could not be set up (the
/// process then simply keeps its old stdout/stderr).
#[cfg(unix)]
pub fn capture(log_file: Option<&Path>) -> Result<(), String> {
    if CAPTURED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let mut file = match log_file {
        Some(p) => {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            OpenOptions::new().create(true).append(true).open(p).ok()
        }
        None => None,
    };
    // SAFETY: plain POSIX fd plumbing on descriptors this process owns.
    let (read_fd, passthrough) = unsafe {
        let passthrough = libc::dup(2);
        let mut fds = [0 as libc::c_int; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            CAPTURED.store(false, Ordering::SeqCst);
            return Err("pipe() failed".into());
        }
        libc::dup2(fds[1], 1);
        libc::dup2(fds[1], 2);
        libc::close(fds[1]);
        (fds[0], passthrough)
    };
    // SAFETY: `read_fd` is the pipe's read end, owned from here on by this `File`.
    let reader = unsafe { File::from_raw_fd(read_fd) };
    let mut pass = if passthrough >= 0 {
        // SAFETY: a fresh duplicate of the original stderr.
        Some(unsafe { File::from_raw_fd(passthrough) })
    } else {
        // Somewhere for fds 1 and 2 to go back to should the reader end (below).
        OpenOptions::new().write(true).open("/dev/null").ok()
    };
    /// Points fds 1 and 2 back at the pass-through when the reader ends or unwinds, before its
    /// read end closes: a write into a pipe nobody drains is EPIPE, which `eprint!` turns into a
    /// panic in whichever thread logs next.
    struct Restore(Option<libc::c_int>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(fd) = self.0 {
                // SAFETY: `fd` is the pass-through, which outlives this guard (captured by the
                // closure, dropped after its locals).
                unsafe {
                    libc::dup2(fd, 1);
                    libc::dup2(fd, 2);
                }
            }
        }
    }
    std::thread::Builder::new()
        .name("inilla-stdio".into())
        .spawn(move || {
            let mut r = BufReader::new(reader);
            // Declared after `r`, so dropped (and run) before `r` closes the pipe.
            let _restore = Restore(pass.as_ref().map(AsRawFd::as_raw_fd));
            let mut raw = Vec::with_capacity(512);
            loop {
                raw.clear();
                match r.read_until(b'\n', &mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                #[cfg(not(target_os = "android"))]
                if let Some(p) = pass.as_mut() {
                    let _ = p.write_all(&raw);
                }
                let line = strip_ansi(&String::from_utf8_lossy(&raw));
                #[cfg(target_os = "android")]
                {
                    let _ = &mut pass;
                    if !is_tracing_line(&line) {
                        crate::android::log::write_line(&line);
                    }
                }
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
    Ok(())
}

/// Vanillish: the same on Windows, through the standard handles rather than descriptors.
#[cfg(windows)]
pub fn capture(log_file: Option<&Path>) -> Result<(), String> {
    if CAPTURED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    crate::windows::stdio::capture(log_file)
        .inspect_err(|_| CAPTURED.store(false, Ordering::SeqCst))
}

/// Elsewhere the process keeps its stdout and stderr.
#[cfg(not(any(unix, windows)))]
pub fn capture(_log_file: Option<&Path>) -> Result<(), String> {
    Err("stdout/stderr capture is only built for unix and Windows targets".into())
}

pub fn is_captured() -> bool {
    CAPTURED.load(Ordering::SeqCst)
}

/// Remove `ESC [ … letter` sequences (the tracing formatter's colours).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A line in tracing-subscriber's fmt shape (`2026-10-15T04:20:00.123Z  INFO target: …`), which
/// bevy_log on Android also writes to logcat by itself.
pub fn is_tracing_line(line: &str) -> bool {
    let b = line.as_bytes();
    let stamp = b.len() > 20
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T';
    if !stamp {
        return false;
    }
    let head = head(line, 48);
    [" TRACE ", " DEBUG ", " INFO ", " WARN ", " ERROR "]
        .iter()
        .any(|l| head.contains(l))
}

/// The logcat priority (`ANDROID_LOG_*`) a captured line deserves.
pub fn logcat_priority(line: &str) -> i32 {
    let head = head(line, 64);
    if head.contains("ERROR") || line.contains("panicked") || head.starts_with("error") {
        6
    } else if head.contains("WARN") || head.starts_with("warning") {
        5
    } else {
        4
    }
}

/// `s` cut to at most `max` bytes, on a char boundary: a byte index inside a multibyte character
/// panics, and this runs on the thread that drains stdout and stderr.
fn head(s: &str, max: usize) -> &str {
    &s[..s.floor_char_boundary(max)]
}

#[cfg(test)]
mod tests {
    use super::{is_tracing_line, logcat_priority, strip_ansi};

    #[test]
    fn tracing_lines_are_told_from_prints() {
        assert!(is_tracing_line(
            "2026-10-15T04:20:00.123456Z  INFO bevy_render::renderer: AdapterInfo"
        ));
        assert!(is_tracing_line(
            "2026-10-15T04:20:00.123456Z  WARN benilla_app::inilla: x"
        ));
        assert!(!is_tracing_line("inilla: game data ready — 12 archives"));
        assert!(!is_tracing_line("2026 was a year"));
        assert_eq!(logcat_priority("thread 'main' panicked at x"), 6);
        assert_eq!(logcat_priority("warning: something"), 5);
        assert_eq!(logcat_priority("inilla: state /x"), 4);
    }

    #[test]
    fn heads_cut_on_char_boundaries() {
        assert_eq!(logcat_priority(&format!("{}— tail", "x".repeat(62))), 4); // '—' at 62..65
        assert_eq!(
            logcat_priority(
                "inilla: no complete game data yet (missing: dbc.MPQ, wmo.MPQ) — setup will ask for it"
            ),
            4
        );
        assert_eq!(
            logcat_priority(
                "inilla: motorola motorola edge 50 ultra · Android 15 (API 35) · SM7550"
            ),
            4
        );
        assert!(is_tracing_line(
            "2026-10-15T04:20:00.123456Z  INFO benilla: Привет мир"
        )); // 'и' at 47..49
    }

    #[test]
    fn colours_are_stripped() {
        assert_eq!(
            strip_ansi("\u{1b}[2m12:00\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m x"),
            "12:00  INFO x"
        );
    }
}
