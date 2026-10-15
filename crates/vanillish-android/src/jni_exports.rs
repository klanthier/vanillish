//! The Java side's `native` methods. JNI finds them by name in `libvanillish.so`
//! (`Java_<package>_<class>_<method>`), so they live in the library's own crate and only forward
//! plain values into `inilla-platform`, which stays package-agnostic. Each runs on Android's UI
//! thread and must not panic: an `extern "system"` function aborts on a panic, so the feeds behind
//! them recover a poisoned lock and never index out of bounds.

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jfloat, jint, JNI_TRUE};
use jni::JNIEnv;

use inilla_platform::gamepad::android as pads;

// GamepadBridge (`android/app/src/main/java/io/github/klanthier/vanillish/GamepadBridge.java`).

#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_GamepadBridge_nativePadConnected(
    mut env: JNIEnv,
    _class: JClass,
    id: jint,
    name: JString,
    vendor: jint,
    product: jint,
    axis_mask: jint,
    centered_mask: jint,
    flat: jfloat,
) {
    let name: String = env
        .get_string(&name)
        .map(Into::into)
        .unwrap_or_else(|_| "Controller".into());
    pads::connect(
        id,
        name,
        vendor as u16,
        product as u16,
        axis_mask as u32,
        centered_mask as u32,
        flat,
    );
}

#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_GamepadBridge_nativePadDisconnected(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
) {
    pads::disconnect(id);
}

#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_GamepadBridge_nativePadKey(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
    key_code: jint,
    down: jboolean,
) {
    pads::key(id, key_code, down == JNI_TRUE);
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_GamepadBridge_nativePadMotion(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
    x: jfloat,
    y: jfloat,
    z: jfloat,
    rz: jfloat,
    rx: jfloat,
    ry: jfloat,
    hat_x: jfloat,
    hat_y: jfloat,
    l_trigger: jfloat,
    r_trigger: jfloat,
    brake: jfloat,
    gas: jfloat,
) {
    pads::motion(
        id,
        [
            x, y, z, rz, rx, ry, hat_x, hat_y, l_trigger, r_trigger, brake, gas,
        ],
    );
}

#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_GamepadBridge_nativePadReleaseAll(
    _env: JNIEnv,
    _class: JClass,
) {
    pads::release_all();
}

// ---- [data] FolderImport (the Storage Access Framework picker) ---------------------------------

use inilla_platform::picker::{android as folders, Picked};

/// A Java string, or empty when it is null or unreadable.
fn plain_string(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s).map(Into::into).unwrap_or_default()
}

/// The player picked a folder (the grant is already taken).
#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_FolderImport_nativeFolderPicked(
    mut env: JNIEnv,
    _class: JClass,
    tree: JString,
    label: JString,
) {
    let tree = plain_string(&mut env, &tree);
    let label = plain_string(&mut env, &label);
    folders::deliver(if tree.is_empty() {
        Picked::Failed("the picker answered without a folder".into())
    } else {
        Picked::Folder { tree, label }
    });
}

#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_FolderImport_nativeFolderPickCancelled(
    _env: JNIEnv,
    _class: JClass,
) {
    folders::deliver(Picked::Cancelled);
}

#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_FolderImport_nativeFolderPickFailed(
    mut env: JNIEnv,
    _class: JClass,
    message: JString,
) {
    let message = plain_string(&mut env, &message);
    folders::deliver(Picked::Failed(if message.is_empty() {
        "the folder picker could not open".into()
    } else {
        message
    }));
}

// ---- [pointer] the hardware mouse and the pen ---------------------------------------------------

use inilla_platform::pointer::{self, HostPointerEvent};

/// A hardware mouse's event (DeX, Bluetooth): px in the window, `BUTTON_*` bits, wheel notches.
#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_PointerBridge_nativeMouse(
    _env: JNIEnv,
    _class: JClass,
    action: jint,
    buttons: jint,
    x: jfloat,
    y: jfloat,
    wheel: jfloat,
) {
    pointer::push(HostPointerEvent::Mouse {
        action,
        buttons: buttons as u32,
        pos: [x, y],
        wheel,
    });
}

/// The captured mouse (pointer capture): motion in the device's counts.
#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_PointerBridge_nativeMouseRelative(
    _env: JNIEnv,
    _class: JClass,
    buttons: jint,
    dx: jfloat,
    dy: jfloat,
    wheel: jfloat,
) {
    pointer::push(HostPointerEvent::Relative {
        buttons: buttons as u32,
        delta: [dx, dy],
        wheel,
    });
}

/// A pen hovering above the screen (px in the window), or gone.
#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_PointerBridge_nativePenHover(
    _env: JNIEnv,
    _class: JClass,
    present: jboolean,
    x: jfloat,
    y: jfloat,
) {
    pointer::push(HostPointerEvent::PenHover(
        (present == JNI_TRUE).then_some([x, y]),
    ));
}

/// A hardware keyboard's key went down.
#[no_mangle]
pub extern "system" fn Java_io_github_klanthier_vanillish_PointerBridge_nativeKeyboardUsed(
    _env: JNIEnv,
    _class: JClass,
) {
    pointer::push(HostPointerEvent::Keyboard);
}
