package io.github.klanthier.vanillish;

import android.app.Activity;
import android.app.ActivityManager;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.Process;
import android.os.VibrationAttributes;
import android.os.VibrationEffect;
import android.os.Vibrator;
import android.os.VibratorManager;
import android.provider.Settings;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Log;
import android.view.Display;
import android.view.Surface;
import android.view.Window;
import android.view.WindowManager;

import java.security.KeyStore;
import java.util.Arrays;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * Everything Rust asks of Android, as static methods (crates/inilla-platform/src/android).
 *
 * <p>Rust calls these from any thread — Bevy's main thread, its task pool, a worker — and never
 * waits on the UI thread: what must happen there is posted to it, and what Android pushes (insets,
 * the fold, the Back key) is cached here for Rust to read. Every method is safe to call before the
 * Activity exists or after it is gone.
 */
public final class Bridge {
    private static final String TAG = "vanillish";

    private Bridge() {}

    static volatile VanillishActivity activity;
    static volatile Context app;

    private static final Handler ui = new Handler(Looper.getMainLooper());

    static void attach(VanillishActivity a) {
        activity = a;
        app = a.getApplicationContext();
    }

    static void detach(VanillishActivity a) {
        if (activity == a) {
            activity = null;
        }
    }

    private static void onUi(Runnable r) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            r.run();
        } else {
            ui.post(r);
        }
    }

    // ---- The window ---------------------------------------------------------------------------

    private static volatile boolean keepScreenOn;

    /** FLAG_KEEP_SCREEN_ON: a controller player never touches the screen. */
    public static void keepScreenOn(boolean on) {
        keepScreenOn = on;
        onUi(Bridge::applyKeepScreenOn);
    }

    static void applyKeepScreenOn() {
        Activity a = activity;
        if (a == null) return;
        Window w = a.getWindow();
        if (keepScreenOn) {
            w.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        } else {
            w.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        }
    }

    private static volatile float frameRate = 0f;

    /**
     * The rate the game renders at (0 = the panel's highest), voted on the game's surface with
     * {@code Surface.setFrameRate} — the call Android recommends for games: the LTPO panel then
     * drops to 60 or 30 with the preset instead of idling at 120 against a capped game, and runs
     * at 120 for High (Samsung's adaptive refresh may otherwise hold a game at 60). Each panel of a
     * foldable has its own modes, so the vote is re-applied on every new surface.
     */
    public static void setFrameRate(float hz) {
        frameRate = hz;
        onUi(Bridge::applyFrameRate);
    }

    /** The highest refresh rate the current panel offers at its current resolution. */
    static float maxRefreshRate(Display d) {
        Display.Mode current = d.getMode();
        float best = current.getRefreshRate();
        for (Display.Mode m : d.getSupportedModes()) {
            if (m.getPhysicalWidth() == current.getPhysicalWidth()
                    && m.getPhysicalHeight() == current.getPhysicalHeight()) {
                best = Math.max(best, m.getRefreshRate());
            }
        }
        return best;
    }

    static void applyFrameRate() {
        VanillishActivity a = activity;
        if (a == null) return;
        Display d = a.getDisplay();
        Surface s = a.gameSurface();
        if (d == null || s == null || !s.isValid()) return;
        float hz = frameRate > 0f ? frameRate : maxRefreshRate(d);
        try {
            s.setFrameRate(hz, Surface.FRAME_RATE_COMPATIBILITY_DEFAULT,
                    Surface.CHANGE_FRAME_RATE_ONLY_IF_SEAMLESS);
        } catch (RuntimeException e) {
            Log.w(TAG, "setFrameRate: " + e);
        }
    }

    /** The panel's refresh rate now, and the highest it offers: {current, max}. */
    public static float[] refreshRate() {
        Activity a = activity;
        Display d = a != null ? a.getDisplay() : null;
        if (d == null) return new float[] {0f, 0f};
        return new float[] {d.getRefreshRate(), maxRefreshRate(d)};
    }

    /** Game Dashboard's mode (GameManager.GAME_MODE_*: 1 standard, 2 performance, 3 battery). */
    static volatile int gameMode;

    public static int gameMode() {
        return gameMode;
    }

    private static volatile int orientationMode = 0;

    /** 0: landscape, either way up. 1: whatever the device is held in. */
    public static void setOrientationMode(int mode) {
        orientationMode = mode;
        onUi(Bridge::applyOrientation);
    }

    static void applyOrientation() {
        Activity a = activity;
        if (a == null) return;
        int want = orientationMode == 1
                ? ActivityInfo.SCREEN_ORIENTATION_FULL_USER
                : ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE;
        if (a.getRequestedOrientation() != want) a.setRequestedOrientation(want);
    }

    // ---- Insets and the fold (pushed by the Activity, read by Rust) ---------------------------

    /** {top, left, bottom, right} in dp, then the camera side (-1 none, 0 left, 1 right). */
    static volatile float[] safeArea = {0f, 0f, 0f, 0f, -1f};

    public static float[] safeArea() {
        return safeArea;
    }

    /**
     * {present, halfOpen, horizontal, left, top, right, bottom (dp, window coordinates),
     * separating, occluding}; present = 0 when no hinge crosses the window.
     */
    static volatile float[] fold = new float[9];

    public static float[] fold() {
        return fold;
    }

    // ---- Back -----------------------------------------------------------------------------------

    private static int backPresses;

    static synchronized void onBack() {
        backPresses++;
    }

    /** Back presses since the last call (each one is the Escape key iOS has no key for either). */
    public static synchronized int takeBackPresses() {
        int n = backPresses;
        backPresses = 0;
        return n;
    }

    // ---- Haptics --------------------------------------------------------------------------------

    private static Vibrator vibrator;
    private static VibrationEffect[] effects;

    /** 0 light, 1 medium, 2 heavy, 3 selection — iOS's impact intensities 0.4 / 0.7 / 1.0. */
    public static void haptic(int kind) {
        Context c = app;
        if (c == null || kind < 0 || kind > 3) return;
        try {
            if (vibrator == null) {
                VibratorManager vm = c.getSystemService(VibratorManager.class);
                vibrator = vm != null ? vm.getDefaultVibrator() : null;
                if (vibrator == null || !vibrator.hasVibrator()) {
                    vibrator = null;
                    return;
                }
                effects = new VibrationEffect[4];
                boolean prims = vibrator.areAllPrimitivesSupported(
                        VibrationEffect.Composition.PRIMITIVE_CLICK,
                        VibrationEffect.Composition.PRIMITIVE_TICK);
                if (prims) {
                    effects[0] = VibrationEffect.startComposition()
                            .addPrimitive(VibrationEffect.Composition.PRIMITIVE_CLICK, 0.4f).compose();
                    effects[1] = VibrationEffect.startComposition()
                            .addPrimitive(VibrationEffect.Composition.PRIMITIVE_CLICK, 0.7f).compose();
                    effects[2] = VibrationEffect.startComposition()
                            .addPrimitive(VibrationEffect.Composition.PRIMITIVE_CLICK, 1.0f).compose();
                    effects[3] = VibrationEffect.startComposition()
                            .addPrimitive(VibrationEffect.Composition.PRIMITIVE_TICK, 0.6f).compose();
                } else {
                    effects[0] = VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK);
                    effects[1] = VibrationEffect.createPredefined(VibrationEffect.EFFECT_CLICK);
                    effects[2] = VibrationEffect.createPredefined(VibrationEffect.EFFECT_HEAVY_CLICK);
                    effects[3] = VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK);
                }
            }
            if (vibrator == null) return;
            if (Build.VERSION.SDK_INT >= 33) {
                vibrator.vibrate(effects[kind],
                        VibrationAttributes.createForUsage(VibrationAttributes.USAGE_TOUCH));
            } else {
                vibrator.vibrate(effects[kind]);
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "haptic: " + e);
        }
    }

    // ---- The clipboard --------------------------------------------------------------------------

    public static String clipboardGet() {
        Context c = app;
        if (c == null) return null;
        try {
            ClipboardManager cm = c.getSystemService(ClipboardManager.class);
            if (cm == null || !cm.hasPrimaryClip()) return null;
            ClipData clip = cm.getPrimaryClip();
            if (clip == null || clip.getItemCount() == 0) return null;
            CharSequence t = clip.getItemAt(0).coerceToText(c);
            return t != null ? t.toString() : null;
        } catch (RuntimeException e) {
            Log.w(TAG, "clipboard: " + e);
            return null;
        }
    }

    public static boolean clipboardSet(String text) {
        Context c = app;
        if (c == null) return false;
        try {
            ClipboardManager cm = c.getSystemService(ClipboardManager.class);
            if (cm == null) return false;
            cm.setPrimaryClip(ClipData.newPlainText("Vanillish", text));
            return true;
        } catch (RuntimeException e) {
            Log.w(TAG, "clipboard: " + e);
            return false;
        }
    }

    // ---- Audio focus ----------------------------------------------------------------------------

    private static volatile boolean audioExclusive;
    private static AudioFocusRequest focus;

    /**
     * Exclusive: ask for AUDIOFOCUS_GAIN, so the player's music pauses (iOS's SoloAmbient). Not:
     * give it back and never ask (Ambient: their music plays under the game).
     */
    public static void audioFocus(boolean exclusive) {
        audioExclusive = exclusive;
        onUi(Bridge::applyAudioFocus);
    }

    static void applyAudioFocus() {
        Context c = app;
        if (c == null) return;
        AudioManager am = c.getSystemService(AudioManager.class);
        if (am == null) return;
        if (audioExclusive) {
            if (focus == null) {
                focus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                        .setAudioAttributes(new AudioAttributes.Builder()
                                .setUsage(AudioAttributes.USAGE_GAME)
                                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                                .build())
                        .setOnAudioFocusChangeListener(change -> {}, ui)
                        .build();
            }
            am.requestAudioFocus(focus);
        } else if (focus != null) {
            am.abandonAudioFocusRequest(focus);
        }
    }

    // ---- Memory ---------------------------------------------------------------------------------

    /** ActivityManager.MemoryInfo: {availMem, threshold, totalMem, lowMemory}. */
    public static long[] memoryInfo() {
        Context c = app;
        if (c == null) return null;
        ActivityManager am = c.getSystemService(ActivityManager.class);
        if (am == null) return null;
        ActivityManager.MemoryInfo mi = new ActivityManager.MemoryInfo();
        am.getMemoryInfo(mi);
        return new long[] {mi.availMem, mi.threshold, mi.totalMem, mi.lowMemory ? 1 : 0};
    }

    // ---- Secrets: AES-256-GCM under a key in the Android Keystore -------------------------------

    private static final String KEY_ALIAS = "vanillish.secrets.v1";
    private static final byte FORMAT = 1;

    private static SecretKey key(boolean create) throws Exception {
        KeyStore ks = KeyStore.getInstance("AndroidKeyStore");
        ks.load(null);
        KeyStore.Entry e = ks.getEntry(KEY_ALIAS, null);
        if (e instanceof KeyStore.SecretKeyEntry) {
            return ((KeyStore.SecretKeyEntry) e).getSecretKey();
        }
        if (!create) return null;
        KeyGenerator kg = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        kg.init(new KeyGenParameterSpec.Builder(KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build());
        return kg.generateKey();
    }

    /** {@code 0x01 | iv[12] | ciphertext+tag}, or null if the Keystore refused. */
    public static synchronized byte[] seal(byte[] plain) {
        try {
            Cipher c = Cipher.getInstance("AES/GCM/NoPadding");
            c.init(Cipher.ENCRYPT_MODE, key(true));
            byte[] iv = c.getIV();
            byte[] body = c.doFinal(plain);
            byte[] out = new byte[1 + iv.length + body.length];
            out[0] = FORMAT;
            System.arraycopy(iv, 0, out, 1, iv.length);
            System.arraycopy(body, 0, out, 1 + iv.length, body.length);
            return out;
        } catch (Exception e) {
            Log.w(TAG, "seal: " + e);
            return null;
        }
    }

    /** The plain bytes, or null (no key any more, a damaged file). */
    public static synchronized byte[] unseal(byte[] sealed) {
        try {
            if (sealed == null || sealed.length < 1 + 12 + 16 || sealed[0] != FORMAT) return null;
            SecretKey k = key(false);
            if (k == null) return null;
            Cipher c = Cipher.getInstance("AES/GCM/NoPadding");
            c.init(Cipher.DECRYPT_MODE, k, new GCMParameterSpec(128, Arrays.copyOfRange(sealed, 1, 13)));
            return c.doFinal(sealed, 13, sealed.length - 13);
        } catch (Exception e) {
            Log.w(TAG, "unseal: " + e);
            return null;
        }
    }

    // ---- The rest of the system -----------------------------------------------------------------

    /** A web page; {@code app-settings:} opens the app's page in the system settings. */
    public static boolean openUrl(String url) {
        Context c = app;
        if (c == null || url == null) return false;
        Intent i;
        if (url.startsWith("app-settings:")) {
            i = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    Uri.fromParts("package", c.getPackageName(), null));
        } else {
            i = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
        }
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            c.startActivity(i);
            return true;
        } catch (ActivityNotFoundException e) {
            return false;
        }
    }

    /**
     * The game quit (its menu's Exit): close the task and end the process — never by returning
     * from android_main into GameActivity's teardown, which a winit app cannot finish.
     */
    public static void quit() {
        onUi(() -> {
            Activity a = activity;
            if (a != null) a.finishAndRemoveTask();
            Process.killProcess(Process.myPid());
        });
    }

    /**
     * Start the game over in a fresh process: the relauncher (its own process) waits for this one
     * to end, then opens the game again. This process ends here.
     */
    public static void relaunch() {
        Context c = app;
        if (c == null) {
            Process.killProcess(Process.myPid());
            return;
        }
        Intent i = new Intent(c, Relauncher.class);
        i.putExtra(Relauncher.EXTRA_PID, Process.myPid());
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        c.startActivity(i);
        Activity a = activity;
        if (a != null) a.finishAffinity();
        Runtime.getRuntime().exit(0);
    }

    // ---- [data] the folder import and sharing files (FolderImport) -----------------------------

    /** Open the folder picker (the Storage Access Framework); false when there is no Activity. */
    public static boolean pickFolder() {
        return FolderImport.pick();
    }

    /** The picked folder's archives as JSON (Rust's import thread); null on failure. */
    public static String listFolder(String tree, int maxDepth) {
        return FolderImport.list(tree, maxDepth);
    }

    /** One archive of the picked folder as a detached file descriptor (Rust owns it); -1 on failure. */
    public static int openDocument(String tree, String docId) {
        return FolderImport.open(tree, docId);
    }

    /** {"tree": …, "label": …} of a picked folder whose copy never finished, or null. */
    public static String pendingFolder() {
        return FolderImport.pendingFolder();
    }

    /** Give a picked folder's grant back and stop offering to resume it. */
    public static void forgetFolder(String tree) {
        FolderImport.forget(tree);
    }

    /** Why the last listFolder or openDocument failed. */
    public static String folderError() {
        return FolderImport.lastError();
    }

    /** Offer one of the app's files (a session log) to another app through the share sheet. */
    public static boolean shareFile(String path, String mime) {
        return FolderImport.share(path, mime);
    }

    // ---- [fold] gesture exclusion and the screen ----------------------------------------------

    /** {left, top, right, bottom} per rect, dp in window coordinates; empty: none. */
    private static volatile float[] gestureExclusion = new float[0];

    /**
     * Where a swipe in from a side edge stays the game's instead of being Android's back gesture:
     * beside the touch controls a thumb rests on (the stick at the left edge). Rects in dp, window
     * coordinates, four floats each; empty clears them (the HUD hid). Applied to the decor view in
     * px on the UI thread, and again whenever the decor view changes size ({@link
     * VanillishActivity}: a fold moves the game to a panel with another density). Android honours
     * 200 dp of height per side edge, and never holds off the bottom edge's home gesture.
     */
    public static void setGestureExclusion(float[] rects) {
        gestureExclusion = rects != null ? rects : new float[0];
        onUi(Bridge::applyGestureExclusion);
    }

    static void applyGestureExclusion() {
        Activity a = activity;
        if (a == null || a.getWindow() == null) return;
        android.view.View decor = a.getWindow().getDecorView();
        float density = a.getResources().getDisplayMetrics().density;
        float[] v = gestureExclusion;
        java.util.List<android.graphics.Rect> rects = new java.util.ArrayList<>(v.length / 4);
        for (int i = 0; i + 3 < v.length; i += 4) {
            rects.add(new android.graphics.Rect(
                    Math.round(v[i] * density), Math.round(v[i + 1] * density),
                    Math.round(v[i + 2] * density), Math.round(v[i + 3] * density)));
        }
        decor.setSystemGestureExclusionRects(rects);
    }

    // ---- [input] the soft keyboard and the host pointer ---------------------------------------

    /**
     * Stay connected: the session service ({@link GameSessionService}) on or off. The game turns
     * it on in the world (with the setting on), while on the screen — Android allows a foreground
     * service to start only then — and off when the player leaves the world, turns the setting
     * off or quits. The first start asks for the notification permission (Android 13+), once.
     */
    public static void sessionService(boolean on) {
        onUi(() -> {
            Context c = app;
            if (c == null) return;
            if (on) {
                VanillishActivity a = activity;
                if (a != null) a.askToNotify();
                GameSessionService.start(c);
            } else {
                GameSessionService.stop(c);
            }
        });
    }
}
