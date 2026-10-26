//! The clipboard for [`crate::pasteboard`], through arboard (benilla's text boxes' own backend):
//! Unicode text (`CF_UNICODETEXT`), opened per call, from any thread.

pub(crate) fn get() -> Option<String> {
    arboard::Clipboard::new()
        .ok()?
        .get_text()
        .ok()
        .filter(|s| !s.is_empty())
}

pub(crate) fn set(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .is_ok()
}
