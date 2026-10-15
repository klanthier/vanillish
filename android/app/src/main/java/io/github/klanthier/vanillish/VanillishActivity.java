package io.github.klanthier.vanillish;

import android.app.GameManager;
import android.content.pm.ApplicationInfo;
import android.content.res.Configuration;
import android.os.Bundle;
import android.os.Process;
import android.system.ErrnoException;
import android.system.Os;
import android.util.Log;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.View;
import android.view.WindowManager;

import androidx.activity.OnBackPressedCallback;
import androidx.core.graphics.Insets;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

import java.util.regex.Pattern;

/**
 * The game's one Activity: GameActivity loads libvanillish.so and starts android_main on its own
 * thread (crates/vanillish-android); everything here is what Android needs around it.
 *
 * <ul>
 *   <li>Full screen as a game wants it: immersive, drawn under the camera cut-out.</li>
 *   <li>Insets and the fold cached in {@link Bridge} for Rust to read.</li>
 *   <li>Controllers taken before GameActivity sees them ({@link GamepadBridge}).</li>
 *   <li>Back is the game's Escape, never "leave the game".</li>
 *   <li>One game per process: a Bevy app cannot be built twice in one, so this process ends with
 *       the Activity, and an Activity created in a process that already ran the game relaunches
 *       into a fresh one.</li>
 * </ul>
 */
public class VanillishActivity extends GameActivity {
    private static final String TAG = "vanillish";

    /** android_main has been started in this process. */
    private static boolean started;

    private GamepadBridge pads;
    private FoldTracker foldTracker;

    /** Debug builds take benilla's environment knobs as intent extras (adb shell am start --es). */
    private static final Pattern KNOB =
            Pattern.compile("^(WOW_|BENILLA_|INILLA_|VANILLISH_)[A-Z0-9_]*$|^RUST_LOG$|^RUST_BACKTRACE$");

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        Bridge.attach(this);
        if (started) {
            // The game already ran in this process (Android kept it and made a new Activity):
            // start over in a fresh one rather than build a second app here.
            Log.i(TAG, "activity created again in a live process; relaunching");
            Bridge.relaunch();
            return;
        }
        started = true;
        applyDebugKnobs();

        // Sets the window up (onSetUpWindow, below), loads libvanillish.so, starts android_main.
        super.onCreate(savedInstanceState);
        hideSystemBars();

        pads = new GamepadBridge(this);
        foldTracker = new FoldTracker(this);
        // [data] the folder picker registers here (FolderImport).
        // [fold] gesture-exclusion set-up goes here.

        // Back (the key, the gesture, predictive back on Android 16) is Escape in the game: it
        // closes the open window, or opens the game menu — as on every other client.
        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                Bridge.onBack();
            }
        });
    }

    /**
     * GameActivity's own set-up asks for ADJUST_RESIZE at run time, overriding the manifest: the
     * game would shrink (and its automatic UI scale with it) whenever the keyboard opens. The
     * keyboard floats over the game instead; the HUD's chat bar shows the line being typed.
     */
    @Override
    protected void onSetUpWindow() {
        super.onSetUpWindow();
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_HIDDEN
                | WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING);
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        WindowManager.LayoutParams lp = getWindow().getAttributes();
        lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
        getWindow().setAttributes(lp);
    }

    /** The game's surface, for the frame-rate vote ({@link Bridge#setFrameRate}). */
    android.view.Surface gameSurface() {
        return mSurfaceView != null ? mSurfaceView.getHolder().getSurface() : null;
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        super.surfaceChanged(holder, format, width, height);
        // A new surface (resume, a fold to the other panel) forgets the vote.
        Bridge.applyFrameRate();
    }

    private void applyDebugKnobs() {
        if ((getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) == 0) return;
        Bundle extras = getIntent() != null ? getIntent().getExtras() : null;
        if (extras == null) return;
        for (String k : extras.keySet()) {
            String v = extras.getString(k);
            if (v == null || !KNOB.matcher(k).matches()) continue;
            try {
                Os.setenv(k, v, true);
                Log.i(TAG, "env " + k + "=" + v);
            } catch (ErrnoException e) {
                Log.w(TAG, "setenv " + k + ": " + e);
            }
        }
    }

    private void hideSystemBars() {
        WindowInsetsControllerCompat c =
                WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        c.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
        c.hide(WindowInsetsCompat.Type.systemBars());
    }

    @Override
    protected void onStart() {
        super.onStart();
        if (foldTracker != null) foldTracker.start();
    }

    @Override
    protected void onStop() {
        if (foldTracker != null) foldTracker.stop();
        super.onStop();
    }

    @Override
    protected void onResume() {
        super.onResume();
        hideSystemBars();
        Bridge.applyKeepScreenOn();
        Bridge.applyFrameRate();
        Bridge.applyOrientation();
        Bridge.applyAudioFocus();
        // Game Dashboard's Performance / Battery modes (read on every resume, as Android asks).
        GameManager gm = getSystemService(GameManager.class);
        Bridge.gameMode = gm != null ? gm.getGameMode() : 0;
        if (pads != null) pads.sync();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        } else if (pads != null) {
            // Ups can go elsewhere while unfocused (the shade, a fold, Game Booster's panel).
            pads.releaseAll();
        }
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        // A fold or unfold moves the game to the other panel: new density, new cut-out.
        View decor = getWindow().getDecorView();
        WindowInsetsCompat insets = androidx.core.view.ViewCompat.getRootWindowInsets(decor);
        if (insets != null) updateSafeArea(insets);
        Bridge.applyFrameRate();
    }

    @Override
    public WindowInsetsCompat onApplyWindowInsets(View v, WindowInsetsCompat insets) {
        updateSafeArea(insets);
        return super.onApplyWindowInsets(v, insets);
    }

    /**
     * The cut-out (and a waterfall edge) in dp, Bevy's logical pixels. The system bars are hidden,
     * so they do not count — as on iOS, where the home indicator is hidden too.
     */
    private void updateSafeArea(WindowInsetsCompat insets) {
        float density = getResources().getDisplayMetrics().density;
        Insets cut = insets.getInsets(WindowInsetsCompat.Type.displayCutout());
        float side = -1f;
        if (cut.left > 0 && cut.left >= cut.right) side = 0f;
        else if (cut.right > 0) side = 1f;
        Bridge.safeArea = new float[] {
            cut.top / density, cut.left / density, cut.bottom / density, cut.right / density, side,
        };
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (pads != null) {
            switch (pads.onKey(event)) {
                case GamepadBridge.CONSUMED:
                    return true;
                case GamepadBridge.TO_SYSTEM:
                    // Unhandled on purpose: the system's own fallback (Home for the pad's Home).
                    return false;
                default:
                    break;
            }
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent event) {
        if (pads != null && pads.onMotion(event)) return true;
        // [pointer] a hardware mouse's hover/scroll and a pen's hover go here.
        return super.dispatchGenericMotionEvent(event);
    }

    /** GameActivity copies at most this many pointers of a MotionEvent. */
    private static final int MAX_POINTERS = 8;

    /**
     * A ninth finger is dropped. winit looks up the pointer a POINTER_DOWN/UP is about by its
     * index, and a ninth finger's index is past the eight GameActivity copies: the lookup panics,
     * which ends the game (Bevy #16798). Its moves never reach the game either (beyond the copy).
     */
    @Override
    public boolean dispatchTouchEvent(MotionEvent event) {
        int action = event.getActionMasked();
        if ((action == MotionEvent.ACTION_POINTER_DOWN || action == MotionEvent.ACTION_POINTER_UP)
                && event.getActionIndex() >= MAX_POINTERS) {
            return true;
        }
        // [pointer] a hardware mouse's buttons go here.
        return super.dispatchTouchEvent(event);
    }

    /** The notification permission's request code (distinct from any other area's). */
    private static final int ASK_NOTIFY = 0x5E55;

    /**
     * The session service's notification needs the permission on Android 13+; asked for once,
     * the first time the service starts (the service runs either way).
     */
    void askToNotify() {
        // (Fully qualified, here and below: the import block is every area's to grow.)
        if (android.os.Build.VERSION.SDK_INT < 33) return;
        String permission = android.Manifest.permission.POST_NOTIFICATIONS;
        if (checkSelfPermission(permission) == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            return;
        }
        android.content.SharedPreferences prefs =
                getSharedPreferences("vanillish.session", MODE_PRIVATE);
        if (prefs.getBoolean("asked_notifications", false)) return;
        prefs.edit().putBoolean("asked_notifications", true).apply();
        requestPermissions(new String[] {permission}, ASK_NOTIFY);
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
        if (requestCode == ASK_NOTIFY && results.length > 0
                && results[0] == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            // The service's notification, posted before the answer, shows now.
            GameSessionService.refresh(this);
        }
    }

    @Override
    protected void onDestroy() {
        if (pads != null) pads.close();
        if (foldTracker != null) foldTracker.stop();
        boolean recreating = isChangingConfigurations();
        Bridge.detach(this);
        // Never call super.onDestroy(): GameActivity's terminateNativeCode waits — with no
        // timeout — for android_main to return, and a winit app never returns on Destroy: the UI
        // thread would hang forever. The game cannot outlive its Activity anyway, and a later
        // Activity must not start a second one in this process: end the process here. (What has
        // to survive was saved when the surface went, on Bevy's Suspended.) If Android is
        // recreating the Activity (a configuration change the manifest did not list), come
        // straight back in a fresh process.
        if (recreating) {
            Bridge.relaunch();
        } else {
            Process.killProcess(Process.myPid());
        }
    }
}
