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

// ---- [pointer] the hardware mouse and the pen ---------------------------------------------------
