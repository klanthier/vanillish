//! Vanillish: **the Windows window** — Vanillish for Windows' name on the window, its icon (the
//! exe's own, resource 1) in the title bar and the taskbar, and its `HWND` for the folder dialog
//! to sit on. Everything here waits for benilla-app's `vanillish` feature: benilla's own desktop
//! run (`cargo run -p benilla`) keeps benilla's window.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::ui::words;

pub(crate) struct WindowsPlugin;

impl Plugin for WindowsPlugin {
    fn build(&self, app: &mut App) {
        if !words::WINDOWS {
            return;
        }
        // Before the event loop makes the OS window from it, so the taskbar never shows the
        // engine's "benilla" (the primary window exists since `WindowPlugin`'s build).
        let world = app.world_mut();
        let mut primary = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
        for mut w in primary.iter_mut(world) {
            w.title = words::APP.to_string();
        }
        #[cfg(all(windows, feature = "vanillish"))]
        app.add_systems(Update, adopt_window);
        app.add_systems(Update, watch_the_gpu)
            .add_systems(Last, log_the_end);
    }
}

/// The session log says why the app ended, which a window that just closes does not.
fn log_the_end(mut exits: MessageReader<AppExit>) {
    for exit in exits.read() {
        info!("vanillish: the app is ending: {exit:?}");
    }
}

/// The session log says when and why the GPU device was lost: the render systems' panics that
/// follow it name only themselves. Once, when the render device exists.
fn watch_the_gpu(device: Option<Res<bevy::render::renderer::RenderDevice>>, mut done: Local<bool>) {
    let Some(device) = device.filter(|_| !*done) else {
        return;
    };
    device
        .wgpu_device()
        .set_device_lost_callback(|reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                error!("vanillish: the GPU device was lost ({reason:?}): {message}");
            }
        });
    *done = true;
}

/// The exe's icon on the window (`ICON_SMALL`, the title bar) and in the taskbar (`ICON_BIG`), and
/// the window's `HWND` handed to the folder picker, once winit has made the window (it is
/// `WINIT_WINDOWS`', which lives on the main thread).
#[cfg(all(windows, feature = "vanillish"))]
fn adopt_window(
    windows: Query<Entity, With<PrimaryWindow>>,
    _main: bevy::ecs::system::NonSendMarker,
    mut done: Local<bool>,
) {
    use winit::platform::windows::{IconExtWindows, WindowExtWindows};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Icon;

    if *done {
        return;
    }
    let Ok(entity) = windows.single() else {
        return;
    };
    *done = bevy::winit::WINIT_WINDOWS.with_borrow(|winit_windows| {
        let Some(window) = winit_windows.get_window(entity) else {
            return false;
        };
        // `1 ICON "Vanillish.ico"` in the exe's resources (`vanillish-windows`' build script).
        match Icon::from_resource(1, None) {
            Ok(icon) => {
                window.set_window_icon(Some(icon.clone()));
                window.set_taskbar_icon(Some(icon));
            }
            Err(e) => warn!("vanillish: no icon in the exe: {e}"),
        }
        match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => {
                inilla_platform::picker::set_owner_window(h.hwnd.get())
            }
            _ => warn!("vanillish: the window has no HWND; the folder dialog opens on its own"),
        }
        true
    });
}
