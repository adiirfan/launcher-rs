package dev.launcherrs.app;

import android.app.NativeActivity;
import android.graphics.PixelFormat;
import android.os.Bundle;

/** Android needs a Java class to start. All logic lives in Rust (liblauncher). */
public class MainActivity extends NativeActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // NativeActivity picks an opaque format; let the wallpaper show through.
        getWindow().setFormat(PixelFormat.TRANSLUCENT);
    }
}
