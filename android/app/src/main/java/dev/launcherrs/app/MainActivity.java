package dev.launcherrs.app;

import android.graphics.PixelFormat;
import android.os.Bundle;

import com.google.androidgamesdk.GameActivity;

/** Android needs a Java class to start. All logic lives in Rust (liblauncher). */
public class MainActivity extends GameActivity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // Let the wallpaper show through wherever Rust draws nothing.
        mSurfaceView.getHolder().setFormat(PixelFormat.TRANSLUCENT);
        mSurfaceView.setZOrderOnTop(true);
    }
}
