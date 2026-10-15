package io.github.klanthier.vanillish;

import android.app.Activity;
import android.graphics.Bitmap;
import android.os.Build;
import android.util.Log;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.PointerIcon;
import android.view.View;

/**
 * A hardware mouse (Samsung DeX, a Bluetooth mouse or trackpad) and a hovering pen, taken before
 * GameActivity sees them.
 *
 * <p>winit has no mouse on Android. GameActivity's input filter lets a mouse's events through as
 * pointer events, and winit makes a finger of a held button and drops the hover, the wheel and
 * which button it was. Android itself turns a mouse's back button into the Back key. So the
 * Activity hands every event here first. A mouse's events and a pen's hover go to Rust (through
 * crates/vanillish-android's JNI shims into inilla-platform's pointer queue, px in the window's
 * coordinates) and are consumed. A pen touching the screen stays a touch. A hardware keyboard's
 * keys are noted, so the touch HUD can step aside while it is in use.
 *
 * <p>The other way, Rust asks through {@link Bridge} for pointer capture (benilla's mouse-look:
 * the mouse's raw motion, with the pointer hidden) and for the pointer's icon (the 1.12 cursors,
 * or none). Everything here runs on the UI thread, except the icon's bitmap, which is built on the
 * caller's thread.
 */
final class PointerBridge {
    private static final String TAG = "vanillish";

    private PointerBridge() {}

    /**
     * A mouse event: its action, every button held after it (BUTTON_*), where it is (px, the
     * window's), and the wheel's notches (+ away from the player).
     */
    static native void nativeMouse(int action, int buttons, float x, float y, float wheel);

    /**
     * The captured mouse: its motion since the last event (the device's counts), the buttons, the
     * wheel.
     */
    static native void nativeMouseRelative(int buttons, float dx, float dy, float wheel);

    /** A pen hovering above the screen at (x, y) px, or gone (present = false). */
    static native void nativePenHover(boolean present, float x, float y);

    /** A key of a hardware keyboard went down: the touch HUD may step aside. */
    static native void nativeKeyboardUsed();

    /** A trackpad's swipe this long (dp) is one notch of a wheel. */
    private static final float SWIPE_DP_PER_NOTCH = 40f;

    /** The last mouse event came from a trackpad (it moves a pointer, but it is not a mouse). */
    private static volatile boolean trackpad;

    /** Where the pointer was last reported (px). */
    private static float lastX;
    private static float lastY;

    // ---- Events in ------------------------------------------------------------------------------

    /** dispatchTouchEvent and dispatchGenericMotionEvent: true when the event was taken. */
    static boolean onMotion(MotionEvent e) {
        if (e.getPointerCount() < 1) return false;
        if (e.isFromSource(InputDevice.SOURCE_MOUSE)) {
            mouse(e);
            return true;
        }
        int tool = e.getToolType(0);
        if (tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER) {
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_HOVER_ENTER:
                case MotionEvent.ACTION_HOVER_MOVE:
                    nativePenHover(true, e.getX(), e.getY());
                    return true;
                case MotionEvent.ACTION_HOVER_EXIT:
                    nativePenHover(false, e.getX(), e.getY());
                    return true;
                default:
                    return false;
            }
        }
        return false;
    }

    private static void mouse(MotionEvent e) {
        int action = e.getActionMasked();
        // A trackpad moves the pointer with a finger (Android 14 reports its pointer as a mouse's).
        trackpad = e.getToolType(0) == MotionEvent.TOOL_TYPE_FINGER
                || e.isFromSource(InputDevice.SOURCE_TOUCHPAD);
        float wheel = 0f;
        if (action == MotionEvent.ACTION_SCROLL) {
            wheel = e.getAxisValue(MotionEvent.AXIS_VSCROLL);
        }
        if (Build.VERSION.SDK_INT >= 34) {
            int c = e.getClassification();
            if (c == MotionEvent.CLASSIFICATION_TWO_FINGER_SWIPE) {
                // A trackpad's two-finger swipe comes as a fake finger dragged along from the
                // pointer, its scroll in px; the pointer itself stays where it was. Fingers moving
                // down scroll the content down, as the wheel turned away does.
                float px = -e.getAxisValue(MotionEvent.AXIS_GESTURE_SCROLL_Y_DISTANCE);
                nativeMouse(MotionEvent.ACTION_SCROLL, e.getButtonState(), lastX, lastY,
                        px / (SWIPE_DP_PER_NOTCH * density()));
                return;
            }
            if (c != MotionEvent.CLASSIFICATION_NONE) {
                // The trackpad's other gestures (a pinch, three or four fingers): the system's.
                return;
            }
        }
        lastX = e.getX();
        lastY = e.getY();
        nativeMouse(action, e.getButtonState(), lastX, lastY, wheel);
    }

    /**
     * dispatchKeyEvent: true when the key was taken. A mouse's back and forward buttons arrive as
     * its buttons (BUTTON5 and BUTTON4 in the game). The Back and Forward keys Android also makes
     * of them are dropped, so the side button is never Escape. With predictive back (Android 16)
     * the Back key goes to the back callback before it gets here; Rust then takes that press back
     * (inilla-platform's MouseBackKeys). The captured mouse's keys come from its relative source;
     * they are dropped too (Rust counts a Back key's press, which comes with the button's, before
     * the release it could be matched to). A hardware keyboard's key is noted.
     */
    static boolean onKey(KeyEvent e) {
        int code = e.getKeyCode();
        if ((code == KeyEvent.KEYCODE_BACK || code == KeyEvent.KEYCODE_FORWARD)
                && (e.isFromSource(InputDevice.SOURCE_MOUSE)
                        || e.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE))) {
            return true;
        }
        if (e.getAction() == KeyEvent.ACTION_DOWN && e.getRepeatCount() == 0
                && isHardwareKeyboard(e.getDevice())) {
            nativeKeyboardUsed();
        }
        return false;
    }

    /**
     * A real keyboard with letters: not the soft keyboard's virtual device, not a remote, not the
     * volume keys.
     */
    private static boolean isHardwareKeyboard(InputDevice d) {
        return d != null && !d.isVirtual()
                && d.getKeyboardType() == InputDevice.KEYBOARD_TYPE_ALPHABETIC;
    }

    /**
     * The captured mouse (pointer capture). A trackpad's captured events (finger positions) are not
     * taken.
     */
    private static final View.OnCapturedPointerListener CAPTURED = (view, e) -> {
        if (!e.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE)) return false;
        // Every sample is a delta; a batched event holds the earlier ones as its history.
        float dx = e.getX();
        float dy = e.getY();
        for (int i = 0; i < e.getHistorySize(); i++) {
            dx += e.getHistoricalX(i);
            dy += e.getHistoricalY(i);
        }
        float wheel = e.getActionMasked() == MotionEvent.ACTION_SCROLL
                ? e.getAxisValue(MotionEvent.AXIS_VSCROLL)
                : 0f;
        nativeMouseRelative(e.getButtonState(), dx, dy, wheel);
        return true;
    };

    // ---- Pointer capture (Bridge.setPointerCapture) ---------------------------------------------

    static volatile boolean wantCapture;
    private static View watchedDecor;

    /**
     * On the UI thread: capture on or off, as last asked. Captured events go to the focused view
     * (the game's surface, which GameActivity focuses), so the listener goes there. It replaces
     * GameActivity's own, which hands them to the native side, whose input filter drops relative
     * motion anyway. Android grants capture only to a focused window and ends it when focus goes;
     * it is asked for again when focus comes back. A trackpad is never captured: it reports finger
     * positions, not motion, and mouse-look then follows its pointer.
     */
    static void applyCapture() {
        Activity a = Bridge.activity;
        if (a == null || a.getWindow() == null) return;
        View decor = a.getWindow().getDecorView();
        watchFocus(decor);
        if (wantCapture && !trackpad) {
            View target = decor.findFocus();
            if (target == null) {
                decor.requestFocus();
                target = decor.findFocus();
            }
            if (target == null) target = decor;
            target.setOnCapturedPointerListener(CAPTURED);
            if (a.hasWindowFocus() && !target.hasPointerCapture()) target.requestPointerCapture();
        } else if (decor.hasPointerCapture()) {
            decor.releasePointerCapture();
        }
    }

    private static void watchFocus(View decor) {
        if (watchedDecor == decor) return;
        watchedDecor = decor;
        decor.getViewTreeObserver().addOnWindowFocusChangeListener(hasFocus -> {
            if (hasFocus && wantCapture) applyCapture();
        });
    }

    // ---- The pointer's icon (Bridge.setPointerIcon, Bridge.setPointerVisible) -------------------

    /** The game's cursor; null: the system's arrow. */
    private static volatile PointerIcon icon;
    private static volatile boolean visible = true;

    /**
     * The cursor's image (ARGB_8888 colours, rows from the top) and hotspot, or null for the
     * system's arrow. Scaled by whole steps of the screen's density (the 1.12 cursors are 32 px,
     * small on a phone's screen), without smoothing.
     */
    static void setIcon(int[] argb, int w, int h, int hotX, int hotY) {
        PointerIcon made = null;
        if (argb != null && w > 0 && h > 0 && argb.length >= w * h) {
            try {
                Bitmap b = Bitmap.createBitmap(argb, w, h, Bitmap.Config.ARGB_8888);
                int s = Math.max(1, (int) density());
                if (s > 1) b = Bitmap.createScaledBitmap(b, w * s, h * s, false);
                float x = Math.min(Math.max(hotX, 0), w - 1) * s;
                float y = Math.min(Math.max(hotY, 0), h - 1) * s;
                made = PointerIcon.create(b, x, y);
            } catch (RuntimeException e) {
                Log.w(TAG, "pointer icon: " + e);
            }
        }
        icon = made;
    }

    static void setVisible(boolean v) {
        visible = v;
    }

    /**
     * On the UI thread: the icon on the decor view. The game's views set none of their own, so it
     * is the one Android resolves under the pointer; TYPE_NULL hides the pointer.
     */
    static void applyIcon() {
        Activity a = Bridge.activity;
        if (a == null || a.getWindow() == null) return;
        PointerIcon shown = visible ? icon : PointerIcon.getSystemIcon(a, PointerIcon.TYPE_NULL);
        a.getWindow().getDecorView().setPointerIcon(shown);
    }

    private static float density() {
        Activity a = Bridge.activity;
        return a != null ? a.getResources().getDisplayMetrics().density : 1f;
    }
}
