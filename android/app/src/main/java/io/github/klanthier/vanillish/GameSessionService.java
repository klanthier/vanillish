package io.github.klanthier.vanillish;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

/**
 * Stay connected: a foreground service that keeps the game's process — and with it the game's
 * connection to its server — alive while the game is in the background.
 *
 * <p>Android freezes a backgrounded app's process within seconds and may reclaim it, and the
 * server drops a character whose client stops answering, so without this a look at another app or
 * a fold to the lock screen ends the session. The game runs it while the player is in the world
 * and the setting ({@code stay_connected}) is on (Rust: {@code device::set_session_service}, via
 * {@link Bridge#sessionService}). Nothing runs in the service itself: the world stops updating
 * while the game is off the screen, but the game's network thread keeps the connection.
 *
 * <p>The type is {@code specialUse} (the manifest says why); the notification is the one Android
 * requires, on a quiet channel, and tapping it returns to the game. If the player declined
 * notifications (Android 13+), the service runs all the same and shows only in the task manager.
 */
public final class GameSessionService extends Service {
    private static final String TAG = "vanillish";
    private static final String CHANNEL = "session";
    private static final int NOTIFICATION = 1;

    // UI thread only: Bridge posts there, and a service's callbacks run there.
    /** The game wants the service. */
    private static boolean wanted;
    /** Asked to start, and not in the foreground yet. */
    private static boolean pending;
    /** In the foreground. */
    private static boolean running;

    /** Start (or refresh) the service. Only allowed while the game is on the screen. */
    static void start(Context c) {
        wanted = true;
        try {
            c.startForegroundService(new Intent(c, GameSessionService.class));
            pending = true;
        } catch (RuntimeException e) {
            // ForegroundServiceStartNotAllowedException: the game is no longer in the foreground.
            Log.w(TAG, "session service: " + e);
        }
    }

    /**
     * Stop the service. One still starting stops itself once in the foreground: Android ends the
     * app when a service it was asked to start in the foreground is stopped before it got there.
     */
    static void stop(Context c) {
        wanted = false;
        if (!pending) c.stopService(new Intent(c, GameSessionService.class));
    }

    /** Post the notification again if the service runs (it may show now: permission granted). */
    static void refresh(Context c) {
        if (running && wanted) start(c);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        pending = false;
        try {
            Notification n = notification();
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                // Android 14+ wants the type, one the manifest declares.
                startForeground(NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            } else {
                // Android 12 and 13 have no specialUse constant: the manifest's type stands.
                startForeground(NOTIFICATION, n);
            }
            running = true;
            NotificationManager nm = getSystemService(NotificationManager.class);
            if (nm != null && !nm.areNotificationsEnabled()) {
                // Declined (Android 13+'s POST_NOTIFICATIONS) or switched off: startForeground
                // needs no permission for its notification, which just does not show. The
                // service runs all the same; the task manager lists it.
                Log.i(TAG, "session service: running, its notification hidden (notifications off)");
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "session service: " + e);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (!wanted) stopSelf();
        // Never brought back by itself: without the game's process there is no session to keep.
        return START_NOT_STICKY;
    }

    @Override
    public void onDestroy() {
        running = false;
        pending = false;
        super.onDestroy();
    }

    @Override
    public void onTaskRemoved(Intent rootIntent) {
        // Swiped away from Recents: the game is gone, and so is the session.
        wanted = false;
        stopSelf();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private Notification notification() {
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm != null) {
            // Low importance: no sound, no pop-up, only the shade (creating it again is a no-op).
            NotificationChannel ch = new NotificationChannel(
                    CHANNEL, "Game connection", NotificationManager.IMPORTANCE_LOW);
            ch.setDescription("Shown while Vanillish keeps your game session open in the background.");
            ch.setShowBadge(false);
            nm.createNotificationChannel(ch);
        }
        // The launcher's own intent: the game's task comes back to the front as it was.
        Intent open = new Intent(this, VanillishActivity.class)
                .setAction(Intent.ACTION_MAIN)
                .addCategory(Intent.CATEGORY_LAUNCHER)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED);
        PendingIntent tap = PendingIntent.getActivity(this, 0, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        // The small icon is a white glyph (the status bar draws its alpha only); the launcher
        // icon's layers are a colored 108 dp canvas, and the adaptive icon is no small icon.
        return new Notification.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_vanillish)
                .setColor(0xFFD4AF37) // iNilla's gold, the shade's accent
                .setContentTitle(getString(R.string.app_name))
                .setContentText("Staying connected to the game server. Tap to return.")
                .setContentIntent(tap)
                .setOngoing(true)
                .setShowWhen(false)
                .setCategory(Notification.CATEGORY_SERVICE)
                .build();
    }
}
