package io.github.klanthier.vanillish;

import android.content.Context;
import android.hardware.input.InputManager;
import android.os.Handler;
import android.os.Looper;
import android.util.SparseBooleanArray;
import android.view.InputDevice;
import android.view.InputEvent;
import android.view.KeyEvent;
import android.view.MotionEvent;

/**
 * Game controllers, taken before GameActivity sees them. GameActivity's input filter drops
 * joystick motion and passes pad buttons to winit as unidentified keys, so the Activity hands
 * every event from a pad here first: buttons and axes go to Rust (crates/vanillish-android's JNI
 * shims → inilla-platform's controller store, read once a frame like iOS's GameController) and are
 * consumed, so no fallback key (B → Back, A → Enter) and no synthetic d-pad is ever generated.
 *
 * <p>Events are matched by device id, so a Bluetooth keyboard's arrow keys still reach the game.
 */
final class GamepadBridge implements InputManager.InputDeviceListener {
    static final int NOT_MINE = 0;
    static final int CONSUMED = 1;
    static final int TO_SYSTEM = 2;

    static native void nativePadConnected(int id, String name, int vendor, int product,
            int axisMask, int centeredMask, float flat);

    static native void nativePadDisconnected(int id);

    static native void nativePadKey(int id, int keyCode, boolean down);

    static native void nativePadMotion(int id, float x, float y, float z, float rz, float rx,
            float ry, float hatX, float hatY, float lTrigger, float rTrigger, float brake, float gas);

    static native void nativePadReleaseAll();

    /** The axes Rust reads, in nativePadMotion's order. */
    private static final int[] AXES = {
        MotionEvent.AXIS_X, MotionEvent.AXIS_Y, MotionEvent.AXIS_Z, MotionEvent.AXIS_RZ,
        MotionEvent.AXIS_RX, MotionEvent.AXIS_RY, MotionEvent.AXIS_HAT_X, MotionEvent.AXIS_HAT_Y,
        MotionEvent.AXIS_LTRIGGER, MotionEvent.AXIS_RTRIGGER, MotionEvent.AXIS_BRAKE,
        MotionEvent.AXIS_GAS,
    };
    /** The stick axes whose "flat" (hardware dead zone) Rust applies. */
    private static final int STICKS = 6;

    private final InputManager im;
    private final SparseBooleanArray pads = new SparseBooleanArray();

    GamepadBridge(Context context) {
        im = (InputManager) context.getSystemService(Context.INPUT_SERVICE);
        if (im != null) im.registerInputDeviceListener(this, new Handler(Looper.getMainLooper()));
        sync();
    }

    void close() {
        if (im != null) im.unregisterInputDeviceListener(this);
    }

    void releaseAll() {
        nativePadReleaseAll();
    }

    /**
     * A pad is a gamepad or a joystick, not a keyboard or a remote that only reports a d-pad. A
     * DualSense's touchpad and motion sensors are separate devices and do not qualify.
     */
    static boolean isPad(InputDevice d) {
        if (d == null || d.isVirtual()) return false;
        int s = d.getSources();
        return (s & InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD
                || (s & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK;
    }

    /** Reconcile with the devices present now (at start, and after a pause). */
    void sync() {
        SparseBooleanArray present = new SparseBooleanArray();
        for (int id : InputDevice.getDeviceIds()) {
            InputDevice d = InputDevice.getDevice(id);
            if (isPad(d)) {
                present.put(id, true);
                if (!pads.get(id)) connect(d);
            }
        }
        for (int i = pads.size() - 1; i >= 0; i--) {
            int id = pads.keyAt(i);
            if (!present.get(id)) {
                pads.delete(id);
                nativePadDisconnected(id);
            }
        }
    }

    private void connect(InputDevice d) {
        int axisMask = 0;
        int centeredMask = 0;
        float flat = 0f;
        for (int i = 0; i < AXES.length; i++) {
            InputDevice.MotionRange r = d.getMotionRange(AXES[i], InputDevice.SOURCE_JOYSTICK);
            if (r == null) continue;
            axisMask |= 1 << i;
            if (r.getMin() < 0f) centeredMask |= 1 << i;
            if (i < STICKS) {
                float range = r.getRange();
                float f = range > 0f ? r.getFlat() / (range / 2f) : 0f;
                flat = Math.max(flat, f);
            }
        }
        pads.put(d.getId(), true);
        nativePadConnected(d.getId(), d.getName(), d.getVendorId(), d.getProductId(), axisMask,
                centeredMask, Math.min(flat, 0.5f));
    }

    @Override
    public void onInputDeviceAdded(int id) {
        InputDevice d = InputDevice.getDevice(id);
        if (isPad(d)) connect(d);
    }

    @Override
    public void onInputDeviceChanged(int id) {
        InputDevice d = InputDevice.getDevice(id);
        if (isPad(d)) {
            connect(d);
        } else {
            onInputDeviceRemoved(id);
        }
    }

    @Override
    public void onInputDeviceRemoved(int id) {
        if (pads.get(id)) {
            pads.delete(id);
            nativePadDisconnected(id);
        }
    }

    private boolean known(InputEvent e) {
        if (pads.get(e.getDeviceId())) return true;
        InputDevice d = e.getDevice();
        if (!isPad(d)) return false;
        connect(d);
        return true;
    }

    private static boolean isNavKey(int code) {
        switch (code) {
            case KeyEvent.KEYCODE_DPAD_UP:
            case KeyEvent.KEYCODE_DPAD_DOWN:
            case KeyEvent.KEYCODE_DPAD_LEFT:
            case KeyEvent.KEYCODE_DPAD_RIGHT:
            case KeyEvent.KEYCODE_DPAD_CENTER:
            case KeyEvent.KEYCODE_DPAD_UP_LEFT:
            case KeyEvent.KEYCODE_DPAD_UP_RIGHT:
            case KeyEvent.KEYCODE_DPAD_DOWN_LEFT:
            case KeyEvent.KEYCODE_DPAD_DOWN_RIGHT:
            case KeyEvent.KEYCODE_BACK:
            case KeyEvent.KEYCODE_MENU:
            case KeyEvent.KEYCODE_MEDIA_RECORD:
                return true;
            default:
                return false;
        }
    }

    int onKey(KeyEvent e) {
        int code = e.getKeyCode();
        if (!(KeyEvent.isGamepadButton(code) || isNavKey(code)) || !known(e)) return NOT_MINE;
        // A key the framework made up from something unconsumed (Samsung turns some pointer
        // motion into d-pad fallbacks): never report it.
        if ((e.getFlags() & KeyEvent.FLAG_FALLBACK) != 0) return CONSUMED;
        int a = e.getAction();
        if (a == KeyEvent.ACTION_DOWN || a == KeyEvent.ACTION_UP) {
            nativePadKey(e.getDeviceId(), code, a == KeyEvent.ACTION_DOWN);
        }
        // The pad's Home goes home, the console way.
        return code == KeyEvent.KEYCODE_BUTTON_MODE ? TO_SYSTEM : CONSUMED;
    }

    boolean onMotion(MotionEvent e) {
        if ((e.getSource() & InputDevice.SOURCE_CLASS_JOYSTICK) == 0 || !known(e)) return false;
        if (e.getActionMasked() == MotionEvent.ACTION_MOVE) {
            nativePadMotion(e.getDeviceId(),
                    e.getAxisValue(AXES[0]), e.getAxisValue(AXES[1]), e.getAxisValue(AXES[2]),
                    e.getAxisValue(AXES[3]), e.getAxisValue(AXES[4]), e.getAxisValue(AXES[5]),
                    e.getAxisValue(AXES[6]), e.getAxisValue(AXES[7]), e.getAxisValue(AXES[8]),
                    e.getAxisValue(AXES[9]), e.getAxisValue(AXES[10]), e.getAxisValue(AXES[11]));
        }
        return true;
    }
}
