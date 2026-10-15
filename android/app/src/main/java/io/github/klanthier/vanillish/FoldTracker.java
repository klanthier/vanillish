package io.github.klanthier.vanillish;

import android.app.Activity;
import android.graphics.Rect;

import androidx.core.util.Consumer;
import androidx.window.java.layout.WindowInfoTrackerCallbackAdapter;
import androidx.window.layout.DisplayFeature;
import androidx.window.layout.FoldingFeature;
import androidx.window.layout.WindowInfoTracker;
import androidx.window.layout.WindowLayoutInfo;

/**
 * The hinge, from Jetpack WindowManager: whether a fold crosses the game's window, whether it is
 * half open, which way it runs and where — cached in {@link Bridge#fold} for Rust (the HUD's
 * tabletop layout: the world above the hinge, the controls below it).
 */
final class FoldTracker implements Consumer<WindowLayoutInfo> {
    private final Activity activity;
    private final WindowInfoTrackerCallbackAdapter tracker;
    private boolean listening;

    FoldTracker(Activity activity) {
        this.activity = activity;
        this.tracker = new WindowInfoTrackerCallbackAdapter(WindowInfoTracker.getOrCreate(activity));
    }

    void start() {
        if (listening) return;
        listening = true;
        tracker.addWindowLayoutInfoListener(activity, Runnable::run, this);
    }

    void stop() {
        if (!listening) return;
        listening = false;
        tracker.removeWindowLayoutInfoListener(this);
    }

    @Override
    public void accept(WindowLayoutInfo info) {
        float density = activity.getResources().getDisplayMetrics().density;
        float[] out = new float[9];
        for (DisplayFeature f : info.getDisplayFeatures()) {
            if (!(f instanceof FoldingFeature)) continue;
            FoldingFeature fold = (FoldingFeature) f;
            Rect b = fold.getBounds();
            out[0] = 1f;
            out[1] = fold.getState() == FoldingFeature.State.HALF_OPENED ? 1f : 0f;
            out[2] = fold.getOrientation() == FoldingFeature.Orientation.HORIZONTAL ? 1f : 0f;
            out[3] = b.left / density;
            out[4] = b.top / density;
            out[5] = b.right / density;
            out[6] = b.bottom / density;
            out[7] = fold.isSeparating() ? 1f : 0f;
            out[8] = fold.getOcclusionType() == FoldingFeature.OcclusionType.FULL ? 1f : 0f;
            break;
        }
        Bridge.fold = out;
    }
}
