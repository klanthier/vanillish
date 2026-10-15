//! **The session log on the device**: the last lines of everything the client printed (the log
//! ring the remote console also reads), filterable, copyable — for the moment something breaks
//! and there is no computer at hand.
//!
//! Vanillish: Android's clipboard goes over binder (about 1 MB a transaction, in UTF-16), so a
//! copy there takes the last lines up to [`CLIP_MAX`] and says so; the whole log goes out through
//! the share sheet instead ([`share_session_log`]).

use std::path::PathBuf;

use bevy_egui::egui;

use super::{words, Overlay};

const SHOWN: usize = 1500;

/// Vanillish: the most a copy puts on Android's clipboard, in bytes of UTF-8 (twice that in the
/// UTF-16 binder carries, well inside its limit; a bigger copy can fail outright).
const CLIP_MAX: usize = 200_000;

pub(super) fn show(ui: &mut egui::Ui, overlay: &mut Overlay) {
    let lines = inilla_core::logring::global().tail(SHOWN * 4);
    ui.horizontal(|ui| {
        ui.label("Filter");
        ui.add(
            egui::TextEdit::singleline(&mut overlay.log_filter)
                .hint_text("warn, error, net:, inilla:")
                .desired_width(260.0),
        );
        ui.checkbox(&mut overlay.log_follow, "follow");
        if ui.button("Copy").clicked() {
            let all: Vec<&str> = filtered(&lines, &overlay.log_filter)
                .map(|(_, l)| l.as_str())
                .collect();
            let max = if words::ANDROID { CLIP_MAX } else { usize::MAX };
            let (text, n) = tail_within(&all, max);
            inilla_platform::pasteboard::set(&text);
            if n < all.len() {
                overlay.toast(words::log_copied_last(n));
            } else {
                overlay.toast(words::LOG_COPIED);
            }
        }
        if inilla_platform::picker::can_share() && ui.button("Share…").clicked() {
            share_session_log(overlay);
        }
    });
    let shown: Vec<&(u64, String)> = filtered(&lines, &overlay.log_filter).collect();
    let skip = shown.len().saturating_sub(SHOWN);
    egui::ScrollArea::vertical()
        .stick_to_bottom(overlay.log_follow)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (_, l) in shown.into_iter().skip(skip) {
                let color = if l.contains("ERROR") || l.contains("panicked") {
                    egui::Color32::LIGHT_RED
                } else if l.contains("WARN") {
                    egui::Color32::from_rgb(240, 200, 90)
                } else {
                    egui::Color32::from_gray(210)
                };
                ui.label(egui::RichText::new(l).monospace().size(12.0).color(color));
            }
        });
}

fn filtered<'a>(
    lines: &'a [(u64, String)],
    filter: &'a str,
) -> impl Iterator<Item = &'a (u64, String)> {
    let f = filter.trim().to_ascii_lowercase();
    lines
        .iter()
        .filter(move |(_, l)| f.is_empty() || l.to_ascii_lowercase().contains(&f))
}

/// Vanillish: the last of `lines` that fit in `max` bytes, one per line, oldest first — and how
/// many that is.
fn tail_within(lines: &[&str], max: usize) -> (String, usize) {
    let mut bytes = 0usize;
    let n = lines
        .iter()
        .rev()
        .take_while(|l| {
            bytes = bytes.saturating_add(l.len() + 1);
            bytes <= max
        })
        .count();
    let mut text = String::with_capacity(bytes.min(max));
    for l in &lines[lines.len() - n..] {
        text.push_str(l);
        text.push('\n');
    }
    (text, n)
}

/// Vanillish: this session's log file — the newest `session-*.log` in
/// `benilla-config/Diagnostics` (the names sort by their UTC time stamp).
fn latest_session_log() -> Option<PathBuf> {
    let dir = crate::local_state::home()?.join("Diagnostics");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("session-") && n.ends_with(".log"))
        })
        .max()
}

/// Vanillish: hand this session's log to another app through the share sheet (mail, a chat, a
/// cloud drive) — the whole file, which the clipboard cannot carry.
pub(super) fn share_session_log(overlay: &mut Overlay) {
    match latest_session_log() {
        None => overlay.toast(words::NO_LOG_YET),
        Some(path) => {
            if !inilla_platform::picker::share_file(&path, "text/plain") {
                overlay.toast(words::SHARE_FAILED);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tail_within;

    #[test]
    fn a_capped_copy_keeps_the_last_lines() {
        let lines = ["first", "second", "third"];
        assert_eq!(
            tail_within(&lines, usize::MAX),
            ("first\nsecond\nthird\n".to_string(), 3)
        );
        // "third\n" + "second\n" = 13 bytes; "first\n" would make 19.
        assert_eq!(tail_within(&lines, 13), ("second\nthird\n".to_string(), 2));
        assert_eq!(tail_within(&lines, 3), (String::new(), 0));
        assert_eq!(tail_within(&[], 10), (String::new(), 0));
    }
}
