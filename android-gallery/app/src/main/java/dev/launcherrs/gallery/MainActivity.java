package dev.launcherrs.gallery;

import android.app.NativeActivity;
import android.os.Bundle;

/** Android needs a Java class to start. All logic and drawing live in Rust
 *  (libgallery); this only passes Android's events to Rust and does the few
 *  things Android only allows from Java. */
public class MainActivity extends NativeActivity {
    static { System.loadLibrary("gallery"); } // so Java finds the native methods

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // Bar sizes straight from Android: from Android 15 apps are drawn
        // under the bars, so the content area no longer shows them.
        getWindow().getDecorView().setOnApplyWindowInsetsListener((v, insets) -> {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets i = insets.getInsets(
                        android.view.WindowInsets.Type.systemBars() | android.view.WindowInsets.Type.displayCutout());
                onInsets(i.top, i.bottom, i.left, i.right);
            } else {
                onInsets(insets.getStableInsetTop(), insets.getStableInsetBottom(),
                        insets.getStableInsetLeft(), insets.getStableInsetRight());
            }
            return v.onApplyWindowInsets(insets);
        });
    }

    static native void onInsets(int top, int bottom, int left, int right);
}
