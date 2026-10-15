package io.github.klanthier.vanillish;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.os.Process;

import java.io.File;

/**
 * Starts the game again in a fresh process. It runs in its own process (":relauncher"), waits for
 * the game's to be gone, opens the game, and ends — ProcessPhoenix's trick. A Bevy app can be built
 * only once per process, so "restart" on Android has to mean a new process, and asking Android to
 * open an Activity from a process that is exiting races the exit.
 */
public final class Relauncher extends Activity {
    static final String EXTRA_PID = "pid";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        int pid = getIntent().getIntExtra(EXTRA_PID, -1);
        new Thread(() -> {
            if (pid > 0) {
                File proc = new File("/proc/" + pid);
                // At most 5 s: the game's process ends within milliseconds of asking for this.
                for (int i = 0; i < 100 && proc.exists(); i++) {
                    if (i == 40) Process.killProcess(pid);
                    try {
                        Thread.sleep(50);
                    } catch (InterruptedException e) {
                        break;
                    }
                }
            }
            runOnUiThread(() -> {
                Intent i = new Intent(this, VanillishActivity.class);
                i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
                startActivity(i);
                finish();
                Runtime.getRuntime().exit(0);
            });
        }, "vanillish-relaunch").start();
    }
}
