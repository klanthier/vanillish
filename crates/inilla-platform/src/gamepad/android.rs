//! Vanillish: controllers on Android, fed from the Java side's `GamepadBridge` (through the JNI
//! shims in `vanillish-android`) and read once a frame by [`poll`].
//!
//! (A first cut: connection tracking only. The full mapping — keycodes, axis layouts, the tap
//! latch — lands with the controller work.)

use std::sync::Mutex;

use inilla_core::pad::PadState;

use super::Pad;

struct Dev {
    id: i32,
    name: String,
}

static PADS: Mutex<Vec<Dev>> = Mutex::new(Vec::new());

fn pads() -> std::sync::MutexGuard<'static, Vec<Dev>> {
    PADS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn connect(
    id: i32,
    name: String,
    _vendor: u16,
    _product: u16,
    _axis_mask: u32,
    _centered_mask: u32,
    _flat: f32,
) {
    let mut p = pads();
    p.retain(|d| d.id != id);
    p.push(Dev { id, name });
}

pub fn disconnect(id: i32) {
    pads().retain(|d| d.id != id);
}

pub fn key(_id: i32, _key_code: i32, _down: bool) {}

pub fn motion(_id: i32, _axes: [f32; 12]) {}

pub fn release_all() {}

pub fn poll(out: &mut Vec<Pad>) {
    for d in pads().iter() {
        out.push(Pad {
            id: d.id as u64,
            name: d.name.clone(),
            category: String::new(),
            state: PadState {
                connected: true,
                ..Default::default()
            },
        });
    }
}
