//! **Vanillish**: the Android host's Bevy side — what needs Bevy's `AndroidApp` or the app's
//! schedules:
//!
//! - **the soft keyboard's text** (`host::ime`): GameActivity delivers typing as edits to its own
//!   text buffer (GameTextInput), which winit drops. The bridge reads the buffer each frame and
//!   types what the keyboard commits into the game as the key messages iOS's keyboard produces, so
//!   benilla's text law and egui take it unchanged; the word still being composed is kept apart
//!   ([`ImePreedit`]) for the HUD to show.
//! - **Back** (`winit_keys`, [`back_to_escape`]): the key or the gesture — the screens take theirs
//!   first, the rest is the game's Escape.
//! - **the native window's size** (`host::follow_native_window`): winit loses a resize that lands
//!   while the Activity is paused, and Bevy keeps a stale size across a resume.
//! - **the lifecycle** ([`lifecycle`], every platform): what must survive is saved when the
//!   surface goes, held keys are let go, the sound is muted.
//! - **the frame rate** ([`vote_frame_rate`], [`follow_game_mode`]): the display is asked for the
//!   preset's rate; Game Dashboard's Performance and Battery modes pick the preset.
//! - **staying connected** ([`session_service`]): a foreground service keeps the process, and the
//!   game's connection, alive while the player is in the world and the app is in the background.
//!
//! Everything but `host` and `winit_keys` (Android only) runs on every platform and is a no-op where
//! its OS call is.

use bevy::prelude::*;
use bevy::window::AppLifecycle;

use inilla_core::settings::GameMode as SystemMode;
use inilla_platform::device::{self, GameMode};

use super::platform::{BackPresses, SyntheticKeys};
use super::{store, Inilla};
use crate::char_select::ClientState;

/// The word the soft keyboard is still composing, which is not the game's yet: Gboard and Samsung
/// Keyboard keep the word being typed open (to correct and predict it) until a space, a
/// punctuation mark or a picked suggestion commits it. The HUD's chat bar shows it after the line
/// (`hud::chat_bar`), so the player sees what they type. Always empty off Android.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImePreedit(pub(crate) String);

/// The display's refresh vote: the rate last asked for (`0` = the panel's highest), and whether a
/// new surface needs it again.
#[derive(Resource, Default)]
struct FrameRateVote {
    voted: Option<u32>,
    stale: bool,
}

pub(crate) struct AndroidPlugin;

impl Plugin for AndroidPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ImePreedit>()
            .init_resource::<FrameRateVote>()
            .add_systems(
                PreUpdate,
                (lifecycle, super::synth::release_all.run_if(leaving_screen)),
            )
            .add_systems(Update, (vote_frame_rate, follow_game_mode))
            .add_systems(Last, (back_to_escape, session_service));
        #[cfg(target_os = "android")]
        host::build(app);
    }
}

/// Bevy's `Suspended` is the last update before the surface goes (no update runs until it is
/// back); `WillSuspend` is never seen on Android but means the same.
fn is_leaving(event: &AppLifecycle) -> bool {
    matches!(event, AppLifecycle::WillSuspend | AppLifecycle::Suspended)
}

/// The app is leaving the screen this update.
fn leaving_screen(mut events: MessageReader<AppLifecycle>) -> bool {
    events.read().filter(|e| is_leaving(e)).count() > 0
}

/// The app leaves the screen, or comes back.
///
/// Leaving: iNilla's four stores are saved now, since a backgrounded Android process can be killed
/// without another frame (swiped from Recents, the low-memory killer, the Activity's own end); the
/// held synthetic keys are let go (`synth::release_all`, beside this), so no latched forward
/// survives the gap; and the sound is muted. Android keeps a backgrounded process running — the
/// audio output's callback included — so the music and the ambience would play on behind the lock
/// screen over a world that no longer updates. The mute is benilla's own (the dev chord's), which
/// silences the main track and leaves the mixer and the stream as they are, rather than a new
/// pause in the output layer.
///
/// Coming back: the sound as it was, and the frame-rate vote again for the new surface.
fn lifecycle(
    mut events: MessageReader<AppLifecycle>,
    state: Res<Inilla>,
    mut sound: Option<ResMut<crate::sound::SoundConfig>>,
    mut vote: ResMut<FrameRateVote>,
    mut muted_before: Local<Option<bool>>,
) {
    for event in events.read() {
        match event {
            AppLifecycle::WillSuspend | AppLifecycle::Suspended => {
                store::save_settings(&state.settings);
                store::save_launcher(&state.launcher);
                store::save_layout(&state.layout);
                store::save_pad(&state.pad);
                if let Some(sound) = sound.as_mut() {
                    if muted_before.is_none() {
                        *muted_before = Some(sound.muted);
                        sound.muted = true;
                    }
                }
                info!("inilla: off the screen; settings saved, sound muted");
            }
            AppLifecycle::WillResume | AppLifecycle::Running => {
                if let (Some(sound), Some(muted)) = (sound.as_mut(), muted_before.take()) {
                    sound.muted = muted;
                    info!("inilla: back on the screen");
                }
                vote.stale = true;
            }
            AppLifecycle::Idle => {}
        }
    }
}

/// Ask the display for the preset's frame rate (`0` = the panel's highest: High and Custom), so a
/// 120 Hz panel runs High at 120 and settles to 60 or 30 with the cap instead of refreshing past
/// it. Again on every return to the screen: the vote belongs to the surface, which Android
/// replaces (the Activity re-applies it on each new surface as well).
fn vote_frame_rate(state: Res<Inilla>, mut vote: ResMut<FrameRateVote>) {
    if !super::touch_device() {
        return;
    }
    let cap = state.settings.quality.fps_cap();
    if vote.voted == Some(cap) && !vote.stale {
        return;
    }
    device::set_frame_rate(cap as f32);
    vote.voted = Some(cap);
    vote.stale = false;
}

/// How long after launch Game Dashboard's mode is first read: the preset's CVars are written when
/// the preset moves after the first frame, so a launch-time change must come after it.
const GAME_MODE_FIRST_READ_SECS: f64 = 2.0;

/// Game Dashboard's mode picks the preset: Performance is High, Battery is the battery saver. Only
/// when the mode changes — since it was last seen, which the settings keep, so a launch in the
/// mode it was left in is no change — never over a Custom preset, and not again until the mode
/// changes once more, so a pick the player makes afterwards stands
/// (`InillaSettings::follow_game_mode`). Read once a second (the Activity refreshes it on every
/// resume, as Android asks).
fn follow_game_mode(mut state: ResMut<Inilla>, time: Res<Time<Real>>, mut next: Local<f64>) {
    if !super::touch_device() {
        return;
    }
    let now = time.elapsed_secs_f64();
    if now < next.max(GAME_MODE_FIRST_READ_SECS) {
        return;
    }
    *next = now + 1.0;
    let mode = match device::game_mode() {
        GameMode::Standard => SystemMode::Standard,
        GameMode::Performance => SystemMode::Performance,
        GameMode::Battery => SystemMode::Battery,
    };
    // Only through a mutable borrow when the mode moved: the settings' change detection (and the
    // debounced save behind it) is not woken once a second for nothing.
    if state.settings.last_game_mode == Some(mode) {
        return;
    }
    let current = state.settings.quality;
    if let Some(preset) = state.settings.follow_game_mode(mode) {
        info!(
            "inilla: Game Dashboard's {mode:?} mode: preset {} -> {}",
            current.label(),
            preset.label()
        );
    }
}

/// What is left of the Back presses once the screens took theirs (in `Update`): the game's Escape,
/// which closes the open window or opens the game menu, as the key does on every other client.
fn back_to_escape(mut back: ResMut<BackPresses>, mut keys: ResMut<SyntheticKeys>) {
    if back.0 == 0 {
        return;
    }
    for _ in 0..back.0 {
        keys.tap(KeyCode::Escape);
    }
    back.0 = 0;
}

/// Stay connected: while the player is in the world and the setting is on, the session service
/// keeps the process alive in the background — Android otherwise freezes or reclaims a
/// backgrounded app, and the server drops a character whose client stops answering. No frame runs
/// meanwhile (Bevy stops updating while the surface is away), but the network thread keeps the
/// connection, and what arrives waits for the return. Off when the player leaves the world, turns
/// the setting off or quits.
fn session_service(
    state: Res<Inilla>,
    client: Res<State<ClientState>>,
    mut exit: MessageReader<AppExit>,
    mut on: Local<bool>,
) {
    let quitting = exit.read().count() > 0;
    let want = !quitting && state.settings.stay_connected && *client.get() == ClientState::InWorld;
    if want != *on {
        device::set_session_service(want);
        *on = want;
    }
}

/// The soft keyboard's buffer read as key presses — pure, so it is tested on the host.
///
/// GameTextInput keeps one text buffer per keyboard session, with a composing region over the word
/// the keyboard is still working on. The game holds a copy of what it was sent; each look at the
/// buffer brings that copy up to date with Backspaces (for what the buffer no longer has) and the
/// new characters, and leaves the composing word out until it is committed. So a correction to a
/// word already sent (a keyboard re-opening it on Backspace, a suggestion replacing it) reaches the
/// game as Backspaces and the new letters, as it would from a real keyboard.
#[cfg(any(target_os = "android", test))]
mod buffer {
    /// A key the IME bridge types into the game.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Typed {
        Char(char),
        Backspace,
        Enter,
        Tab,
    }

    /// One look at the buffer.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub(super) struct Step {
        /// What brings the game's copy up to date, in order.
        pub(super) keys: Vec<Typed>,
        /// The buffer's text the game has after `keys`.
        pub(super) sent: String,
        /// The composing word, not sent.
        pub(super) preedit: String,
        /// A line break was typed: the game's line is sent, and the buffer should start over.
        pub(super) line_done: bool,
    }

    /// Bring the game's copy of the buffer, `sent`, up to date with `text`. `compose` is the
    /// composing region in UTF-16 units (Java's, which GameTextInput passes through); `commit_all`
    /// sends the composing word too (the line ends, the box closes, a hardware key typed it).
    /// Characters the game already has are never taken back while the buffer still holds them,
    /// even when the keyboard re-opens them for composing.
    pub(super) fn step(
        sent: &str,
        text: &str,
        compose: Option<(usize, usize)>,
        commit_all: bool,
    ) -> Step {
        let chars: Vec<char> = text.chars().collect();
        let sent: Vec<char> = sent.chars().collect();
        let committed = match compose {
            Some((a, b)) if !commit_all && a != b => char_index(text, a.min(b)),
            _ => chars.len(),
        };
        let common = sent.iter().zip(&chars).take_while(|(a, b)| a == b).count();
        let mut target = committed.max(common);
        // A committed line break ends the line: whatever follows it goes too, so the buffer can
        // start over.
        if chars[common..target]
            .iter()
            .any(|c| matches!(c, '\n' | '\r'))
        {
            target = chars.len();
        }
        let mut keys = vec![Typed::Backspace; sent.len() - common];
        let mut line_done = false;
        let mut after_cr = false;
        for &c in &chars[common..target] {
            match c {
                '\r' => {
                    keys.push(Typed::Enter);
                    line_done = true;
                }
                // `\r\n` is one line break.
                '\n' => {
                    if !after_cr {
                        keys.push(Typed::Enter);
                    }
                    line_done = true;
                }
                '\t' => keys.push(Typed::Tab),
                c if c.is_control() => {}
                c => keys.push(Typed::Char(c)),
            }
            after_cr = c == '\r';
        }
        Step {
            keys,
            sent: chars[..target].iter().collect(),
            preedit: chars[target..].iter().collect(),
            line_done,
        }
    }

    /// The char index at a UTF-16 offset into `text`, clamped to its end.
    fn char_index(text: &str, utf16: usize) -> usize {
        let mut units = 0;
        for (i, c) in text.chars().enumerate() {
            if units >= utf16 {
                return i;
            }
            units += c.len_utf16();
        }
        text.chars().count()
    }

    #[cfg(test)]
    mod tests {
        use super::Typed::{Backspace, Char, Enter, Tab};
        use super::*;

        fn chars(s: &str) -> Vec<Typed> {
            s.chars().map(Char).collect()
        }

        #[test]
        fn committed_text_is_typed_once() {
            let s = step("", "hi", None, false);
            assert_eq!(s.keys, chars("hi"));
            assert_eq!(s.sent, "hi");
            let again = step(&s.sent, "hi", None, false);
            assert!(again.keys.is_empty());
        }

        #[test]
        fn the_composing_word_waits_as_preedit() {
            let s = step("", "hel", Some((0, 3)), false);
            assert!(s.keys.is_empty());
            assert_eq!(s.preedit, "hel");
            let s = step(&s.sent, "hello ", None, false);
            assert_eq!(s.keys, chars("hello "));
            let s = step(&s.sent, "hello w", Some((6, 7)), false);
            assert!(s.keys.is_empty());
            assert_eq!((s.sent.as_str(), s.preedit.as_str()), ("hello ", "w"));
        }

        #[test]
        fn a_deletion_in_the_buffer_is_a_backspace() {
            let s = step("hello", "hell", None, false);
            assert_eq!(s.keys, vec![Backspace]);
            assert_eq!(s.sent, "hell");
        }

        #[test]
        fn a_reopened_word_is_not_taken_back_until_it_changes() {
            // Backspace into a sent word: the keyboard composes it again, the game keeps it.
            let s = step("hello", "hello", Some((0, 5)), false);
            assert!(s.keys.is_empty());
            assert_eq!(s.sent, "hello");
            // A suggestion replaces it: only the diverging tail goes back.
            let s = step("hello", "help", Some((0, 4)), false);
            assert_eq!(s.keys, vec![Backspace, Backspace]);
            assert_eq!((s.sent.as_str(), s.preedit.as_str()), ("hel", "p"));
            let s = step(&s.sent, "help ", None, false);
            assert_eq!(s.keys, chars("p "));
        }

        #[test]
        fn commit_all_sends_the_composition_and_keeps_it_sent() {
            // A hardware key: GameTextInput marks the whole buffer composing.
            let s = step("a", "ab", Some((0, 2)), true);
            assert_eq!(s.keys, chars("b"));
            let s = step(&s.sent, "ab", Some((0, 2)), false);
            assert!(s.keys.is_empty());
            assert_eq!(s.preedit, "");
        }

        #[test]
        fn a_line_break_is_enter_and_ends_the_line() {
            let s = step("", "hi\n", None, false);
            assert_eq!(s.keys, vec![Char('h'), Char('i'), Enter]);
            assert!(s.line_done);
            let s = step("", "a\r\nb", Some((3, 4)), false);
            assert_eq!(s.keys, vec![Char('a'), Enter, Char('b')]);
            assert_eq!(s.preedit, "");
            assert!(s.line_done);
        }

        #[test]
        fn tab_is_its_key_and_controls_are_dropped() {
            let s = step("", "a\tb\u{7}", None, false);
            assert_eq!(s.keys, vec![Char('a'), Tab, Char('b')]);
        }

        #[test]
        fn composing_offsets_are_utf16() {
            // The emoji is two UTF-16 units: the region (2, 4) starts after it.
            let s = step("", "😀ab", Some((2, 4)), false);
            assert_eq!(s.keys, vec![Char('😀')]);
            assert_eq!(s.preedit, "ab");
        }
    }
}

/// winit's key messages, sorted before anything reads them (Android) — plain Bevy apart from the
/// Activity's Back count (a no-op off Android), so it is tested on the host.
///
/// - **Back** (the key, Android 12–15 without predictive back) is counted into [`BackPresses`] and
///   removed: to the game it is an unknown key. The callback's presses (the gesture, Android 16)
///   come from the Activity and are added here too.
/// - **Enter** from a hardware keyboard has `"\n"` for its text and its logical key; it becomes
///   the named key (iOS's Return gets the same rewrite in `platform`).
/// - A **character key's release** is noted (`WinitKeys::typed`) for the IME bridge: while the
///   soft keyboard is up, GameTextInput takes a hardware key's press into its buffer, marked
///   composing, and only the release reaches winit.
#[cfg(any(target_os = "android", test))]
mod winit_keys {
    use bevy::ecs::message::MessageCursor;
    use bevy::input::keyboard::{Key, KeyboardInput};
    use bevy::input::ButtonState;
    use bevy::prelude::*;

    use inilla_platform::device;

    use crate::inilla::platform::BackPresses;

    /// What winit delivered between two frames.
    #[derive(Resource, Default)]
    pub(super) struct WinitKeys {
        /// Moved past the whole queue at the end of every frame ([`keys_seen`]), so [`keys`] reads
        /// only what winit wrote in between — never the app's own keys (the IME bridge's
        /// characters, the synthetic Enter), which it would otherwise take for a hardware key's
        /// release, and write again with a rewrite.
        cursor: MessageCursor<KeyboardInput>,
        /// A character key came up this frame (see the module).
        pub(super) typed: bool,
    }

    pub(super) fn keys(
        mut messages: ResMut<Messages<KeyboardInput>>,
        mut seen: ResMut<WinitKeys>,
        mut back: ResMut<BackPresses>,
    ) {
        let presses = device::take_back_presses();
        if presses > 0 {
            back.0 += presses;
        }
        // A cursor rather than this update's half of the buffer: a message update in `First` (on
        // any frame after a fixed step) has already moved winit's messages to the older half.
        let seen = &mut *seen;
        let fresh: Vec<KeyboardInput> = seen.cursor.read(&messages).cloned().collect();
        seen.typed = fresh.iter().any(|k| {
            k.state == ButtonState::Released && matches!(k.logical_key, Key::Character(_))
        });
        if !fresh.iter().any(|k| is_back(k) || is_newline(k)) {
            return;
        }
        // Messages cannot be removed one by one: drop the buffers (everything but `fresh` was
        // written last frame, and every reader has had it; nothing has read `fresh` yet) and write
        // these back, rewritten.
        messages.drain().for_each(drop);
        for mut k in fresh {
            if is_back(&k) {
                if k.state == ButtonState::Pressed && !k.repeat {
                    back.0 += 1;
                }
                continue;
            }
            if is_newline(&k) {
                if !matches!(k.key_code, KeyCode::Enter | KeyCode::NumpadEnter) {
                    k.key_code = KeyCode::Enter;
                }
                k.logical_key = Key::Enter;
                k.text = None;
            }
            messages.write(k);
        }
        seen.cursor.clear(&messages);
    }

    /// `Last`: everything in the queue now was written by the app this frame.
    pub(super) fn keys_seen(mut seen: ResMut<WinitKeys>, messages: Res<Messages<KeyboardInput>>) {
        seen.cursor.clear(&messages);
    }

    fn is_back(k: &KeyboardInput) -> bool {
        matches!(k.logical_key, Key::BrowserBack)
    }

    fn is_newline(k: &KeyboardInput) -> bool {
        matches!(k.text.as_deref(), Some("\n") | Some("\r"))
            || matches!(&k.logical_key, Key::Character(c) if c.as_str() == "\n" || c.as_str() == "\r")
    }

    #[cfg(test)]
    mod tests {
        use bevy::input::keyboard::NativeKeyCode;

        use super::*;

        fn key(
            code: KeyCode,
            logical: Key,
            state: ButtonState,
            text: Option<&str>,
        ) -> KeyboardInput {
            KeyboardInput {
                key_code: code,
                logical_key: logical,
                state,
                text: text.map(Into::into),
                repeat: false,
                window: Entity::PLACEHOLDER,
            }
        }

        fn typed(c: &str, state: ButtonState) -> KeyboardInput {
            let text = (state == ButtonState::Pressed).then_some(c);
            key(
                KeyCode::Unidentified(NativeKeyCode::Unidentified),
                Key::Character(c.into()),
                state,
                text,
            )
        }

        /// What the app's own producer (the IME bridge) writes after `keys` this frame.
        #[derive(Resource, Default)]
        struct Own(Vec<KeyboardInput>);

        /// Every key message the game read.
        #[derive(Resource, Default)]
        struct Read(Vec<KeyboardInput>);

        fn own(mut own: ResMut<Own>, mut out: MessageWriter<KeyboardInput>) {
            for k in own.0.drain(..) {
                out.write(k);
            }
        }

        fn read(mut keys: MessageReader<KeyboardInput>, mut read: ResMut<Read>) {
            read.0.extend(keys.read().cloned());
        }

        fn app() -> App {
            let mut app = App::new();
            app.add_message::<KeyboardInput>()
                .init_resource::<BackPresses>()
                .init_resource::<WinitKeys>()
                .init_resource::<Own>()
                .init_resource::<Read>()
                .add_systems(PreUpdate, (keys, own).chain())
                .add_systems(Update, read)
                .add_systems(Last, keys_seen);
            app
        }

        /// winit delivers between frames, before `First`.
        fn winit(app: &mut App, k: KeyboardInput) {
            app.world_mut().write_message(k);
        }

        #[test]
        fn the_apps_own_keys_are_not_winits() {
            let mut app = app();
            app.update();
            app.world_mut().resource_mut::<Own>().0 = vec![
                typed("a", ButtonState::Pressed),
                typed("a", ButtonState::Released),
            ];
            app.update();
            // Back arrives: the rewrite must not send the bridge's "a" a second time, and its
            // release is not a hardware key's.
            winit(
                &mut app,
                key(
                    KeyCode::Unidentified(NativeKeyCode::Android(4)),
                    Key::BrowserBack,
                    ButtonState::Pressed,
                    None,
                ),
            );
            app.update();
            assert!(!app.world().resource::<WinitKeys>().typed);
            assert_eq!(app.world().resource::<BackPresses>().0, 1);
            let read = &app.world().resource::<Read>().0;
            assert_eq!(read.len(), 2, "{read:?}");
            assert!(read
                .iter()
                .all(|k| k.logical_key == Key::Character("a".into())));
        }

        #[test]
        fn hardware_enter_is_the_named_key() {
            let mut app = app();
            app.update();
            winit(
                &mut app,
                key(
                    KeyCode::Enter,
                    Key::Character("\n".into()),
                    ButtonState::Pressed,
                    Some("\n"),
                ),
            );
            app.update();
            let read = &app.world().resource::<Read>().0;
            assert_eq!(read.len(), 1);
            assert_eq!(
                (read[0].key_code, &read[0].logical_key, &read[0].text),
                (KeyCode::Enter, &Key::Enter, &None)
            );
        }

        #[test]
        fn a_hardware_character_release_is_noted_for_one_frame() {
            let mut app = app();
            app.update();
            winit(
                &mut app,
                key(
                    KeyCode::KeyA,
                    Key::Character("a".into()),
                    ButtonState::Released,
                    None,
                ),
            );
            app.update();
            assert!(app.world().resource::<WinitKeys>().typed);
            app.update();
            assert!(!app.world().resource::<WinitKeys>().typed);
        }
    }
}

/// The parts that need Android's `AndroidApp` (from `bevy::android::ANDROID_APP`).
#[cfg(target_os = "android")]
mod host {
    use std::collections::VecDeque;

    use bevy::android::android_activity::input::{
        ImeOptions, InputType, TextInputAction, TextInputState,
    };
    use bevy::android::android_activity::AndroidApp;
    use bevy::android::ANDROID_APP;
    use bevy::ecs::system::NonSendMarker;
    use bevy::input::keyboard::{Key, KeyboardInput, NativeKeyCode};
    use bevy::input::ButtonState;
    use bevy::prelude::*;
    use bevy::window::{AppLifecycle, PrimaryWindow};

    use super::buffer::{self, Typed};
    use super::winit_keys::{self, WinitKeys};
    use super::ImePreedit;
    use crate::inilla::platform::{ImePurpose, SyntheticKeys};

    /// A buffer this long with nothing composing starts over (GameTextInput copies it into a
    /// fixed 64 KiB, and every frame's read decodes all of it).
    const LONGEST_LINE: usize = 1024;

    pub(super) fn build(app: &mut App) {
        app.init_resource::<Ime>()
            .init_resource::<WinitKeys>()
            .add_systems(PreUpdate, follow_native_window)
            .add_systems(
                PreUpdate,
                (
                    // First of all readers: it rewrites what winit wrote.
                    winit_keys::keys.before(crate::inilla::InputProducers),
                    // After the controller's taps, ahead of the synthetic keys' presses.
                    ime.after(crate::inilla::InputProducers)
                        .before(crate::inilla::platform::synthetic_keys),
                )
                    .chain()
                    .before(bevy::input::InputSystems),
            )
            .add_systems(Last, winit_keys::keys_seen);
    }

    /// Keep `Window.resolution` at the native window's size. winit sends a resize only while the
    /// Activity is resumed, so one that lands while it is paused (a fold to the other panel) is
    /// lost; Bevy keeps the cached size when it recreates the window on resume; and at launch the
    /// window carries the desktop's boot size until the first resize. The write goes through
    /// bevy_winit like any resize (it asks winit for the size, which answers with the native one),
    /// so the swapchain, the UI and the HUD follow.
    pub(super) fn follow_native_window(
        _main: NonSendMarker,
        mut windows: Query<&mut Window, With<PrimaryWindow>>,
    ) {
        // `None` while the app is off the screen.
        let Some(native) = ANDROID_APP.get().and_then(AndroidApp::native_window) else {
            return;
        };
        let (w, h) = (native.width(), native.height());
        if w <= 0 || h <= 0 {
            return;
        }
        let (w, h) = (w as u32, h as u32);
        let Ok(mut window) = windows.single_mut() else {
            return;
        };
        let now = (
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        );
        if now != (w, h) {
            info!(
                "inilla: the window is {w}x{h} px (Bevy had {}x{})",
                now.0, now.1
            );
            window.resolution.set_physical_resolution(w, h);
        }
    }

    /// The soft keyboard between frames.
    #[derive(Resource, Default)]
    pub(super) struct Ime {
        /// The keyboard is up for the current text focus, and its buffer is read.
        active: bool,
        /// The kind of field the keyboard was last set up for.
        configured: Option<ImePurpose>,
        /// The buffer's text the game has.
        sent: String,
        /// The buffer was told to start over, and may still hold `sent` until Android gets to it.
        clearing: bool,
        /// Keys waiting to be typed into the game; an Enter or a Tab ends a frame's share.
        queue: VecDeque<Typed>,
    }

    /// The soft keyboard's bridge. winit raises and lowers the keyboard with `Window.ime_enabled`
    /// (`keyboard_policy` decides), but drops everything GameActivity reports about the text, so
    /// this side reads GameTextInput's buffer itself:
    ///
    /// - **raised**: the keyboard is set up for the field's kind ([`configure`]) and asked for
    ///   explicitly (winit's request is the implicit kind some keyboards ignore); whatever the
    ///   buffer still holds is not typed. The kind is what the screens left in [`ImePurpose`] last
    ///   frame, so their per-frame reset must not land between their pass and `PreUpdate`: a kind
    ///   that flips between frames restarts the keyboard each time;
    /// - **each frame**: the buffer is read and what it committed is typed into the game
    ///   ([`buffer::step`]) — characters as iOS's keyboard sends them (pressed with their text,
    ///   then released; no key code), deletions as Backspace, a line break as Enter through
    ///   [`SyntheticKeys`] (down this frame, up the next) — while the composing word waits in
    ///   [`ImePreedit`];
    /// - **the line ends** (an Enter or Escape queued: the chat bar's Send or Close, Back), the
    ///   field changes kind, the box loses focus, the app leaves the screen: the composing word is
    ///   sent first, then the buffer starts over; losing focus also hides the keyboard, which also
    ///   stops GameTextInput taking a hardware keyboard's keys.
    ///
    /// The buffer is kept for a whole line, not cleared after each word: Android applies a clear
    /// on its own thread a moment later, and typing that lands in between would be wiped unsent —
    /// a letter lost now and then from a password typed at speed. A line end is the one moment no
    /// typing follows.
    ///
    /// Backspace in an empty buffer is not the keyboard's to apply, so it sends the key, which
    /// reaches winit as Backspace. A hardware keyboard's typing arrives through the buffer too
    /// while the keyboard is up, committed by the key's release ([`WinitKeys::typed`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ime(
        _main: NonSendMarker,
        mut ime: ResMut<Ime>,
        mut preedit: ResMut<ImePreedit>,
        purpose: Res<ImePurpose>,
        mut synthetic: ResMut<SyntheticKeys>,
        winit: Res<WinitKeys>,
        mut out: MessageWriter<KeyboardInput>,
        mut lifecycle: MessageReader<AppLifecycle>,
        windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    ) {
        let Some(app) = ANDROID_APP.get() else {
            return;
        };
        let Ok((window, w)) = windows.single() else {
            return;
        };
        let ime = &mut *ime;
        let leaving = lifecycle.read().filter(|e| super::is_leaving(e)).count() > 0;
        let want = w.ime_enabled && !leaving;

        let kind_moved = ime.configured != Some(*purpose);
        if kind_moved {
            configure(app, *purpose);
            ime.configured = Some(*purpose);
        }

        let mut composing = String::new();
        if ime.active {
            let ends = !want || kind_moved || synthetic.ends_line();
            if let Some(state) = read_state(app) {
                if ime.clearing && (state.text.is_empty() || !state.text.starts_with(&ime.sent)) {
                    // Android has applied the clear (anything here was typed after it).
                    ime.sent.clear();
                    ime.clearing = false;
                }
                let region = state.compose_region.map(|r| (r.start, r.end));
                let step = buffer::step(&ime.sent, &state.text, region, ends || winit.typed);
                ime.queue.extend(step.keys);
                ime.sent = step.sent;
                let long = step.preedit.is_empty() && ime.sent.chars().count() > LONGEST_LINE;
                if ends || step.line_done || long {
                    start_over(app, ime, kind_moved);
                } else {
                    composing = step.preedit;
                }
            } else if ends {
                // An unsettled read (see `read_state`): a composing word is lost, nothing more.
                start_over(app, ime, kind_moved);
            }
            if !want {
                app.hide_soft_input(false);
                ime.active = false;
            }
        } else if want {
            // Whatever the buffer still holds is not the game's (a raise restarts the keyboard,
            // which applies the field's kind).
            ime.sent = read_state(app).map(|s| s.text).unwrap_or_default();
            ime.clearing = false;
            start_over(app, ime, false);
            app.show_soft_input(false);
            ime.active = true;
        }
        if preedit.0 != composing {
            preedit.0 = composing;
        }
        type_queued(&mut ime.queue, &mut out, &mut synthetic, window);
    }

    /// Set the keyboard up for the field ([`ImePurpose`]), as Android's `EditorInfo`: multi-line
    /// text, so the keyboard's Enter types a line break into the buffer (a single-line field's
    /// Enter is an editor action, which winit drops); never the full-screen editor a keyboard
    /// otherwise takes in landscape, over the whole game on the cover screen; a password neither
    /// suggested nor learned, on the plain layout; an address on the URL layout. Applied by the
    /// keyboard's next restart (a buffer clear, a raise).
    fn configure(app: &AndroidApp, purpose: ImePurpose) {
        let mut input = InputType::TYPE_CLASS_TEXT | InputType::TYPE_TEXT_FLAG_MULTI_LINE;
        // android-activity 0.6.1 spells the no-extract-UI flag `IMG_`.
        let mut options = ImeOptions::IME_FLAG_NO_FULLSCREEN | ImeOptions::IMG_FLAG_NO_EXTRACT_UI;
        match purpose {
            ImePurpose::Text => {}
            ImePurpose::Password => {
                input |= InputType::TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                    | InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS;
                options |= ImeOptions::IME_FLAG_NO_PERSONALIZED_LEARNING;
            }
            ImePurpose::Url => input |= InputType::TYPE_TEXT_VARIATION_URI,
        }
        options.set_action(TextInputAction::None);
        app.set_ime_editor_info(input, TextInputAction::None, options);
    }

    /// Start the buffer over. Android applies it on its UI thread a moment later, so until the
    /// buffer is seen empty (or holding something else) `sent` stays what it held, and nothing in
    /// it is typed twice. An empty buffer is left alone — a clear would only race the next key —
    /// unless the keyboard must `restart` to take a new field kind.
    fn start_over(app: &AndroidApp, ime: &mut Ime, restart: bool) {
        if ime.sent.is_empty() {
            ime.clearing = false;
            if restart {
                app.set_text_input_state(TextInputState::default());
            }
        } else if !ime.clearing || restart {
            app.set_text_input_state(TextInputState::default());
            ime.clearing = true;
        }
    }

    /// The buffer, read until two reads agree. GameTextInput copies its state under a lock but
    /// hands the text over from a shared buffer after releasing it, so a read racing the keyboard's
    /// thread can tear; two equal reads in a row did not. `None` if it would not settle.
    fn read_state(app: &AndroidApp) -> Option<TextInputState> {
        let mut last = app.text_input_state();
        for _ in 0..2 {
            let next = app.text_input_state();
            let region = |s: &TextInputState| s.compose_region.map(|r| (r.start, r.end));
            if next.text == last.text && region(&next) == region(&last) {
                return Some(next);
            }
            last = next;
        }
        None
    }

    /// Type the queue into the game, up to and including its first Enter or Tab: those go through
    /// [`SyntheticKeys`], down after this frame's text and up the next frame, before what follows.
    fn type_queued(
        queue: &mut VecDeque<Typed>,
        out: &mut MessageWriter<KeyboardInput>,
        synthetic: &mut SyntheticKeys,
        window: Entity,
    ) {
        while let Some(key) = queue.pop_front() {
            match key {
                Typed::Char(c) => {
                    let mut utf8 = [0u8; 4];
                    let s: &str = c.encode_utf8(&mut utf8);
                    for state in [ButtonState::Pressed, ButtonState::Released] {
                        out.write(KeyboardInput {
                            key_code: KeyCode::Unidentified(NativeKeyCode::Unidentified),
                            logical_key: Key::Character(s.into()),
                            state,
                            text: (state == ButtonState::Pressed).then(|| s.into()),
                            repeat: false,
                            window,
                        });
                    }
                }
                Typed::Backspace => {
                    for state in [ButtonState::Pressed, ButtonState::Released] {
                        out.write(KeyboardInput {
                            key_code: KeyCode::Backspace,
                            logical_key: Key::Backspace,
                            state,
                            text: None,
                            repeat: false,
                            window,
                        });
                    }
                }
                Typed::Enter => {
                    synthetic.tap(KeyCode::Enter);
                    break;
                }
                Typed::Tab => {
                    synthetic.tap(KeyCode::Tab);
                    break;
                }
            }
        }
    }
}
