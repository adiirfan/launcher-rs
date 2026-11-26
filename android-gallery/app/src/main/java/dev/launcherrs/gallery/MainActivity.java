package dev.launcherrs.gallery;

import android.Manifest;
import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.ContentObserver;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;
import android.provider.Settings;

import java.util.ArrayList;

/** Android needs a Java class to start. All logic and drawing live in Rust
 *  (libgallery); this only passes Android's events to Rust and does the few
 *  things Android only allows from Java. Rust calls the public methods. */
public class MainActivity extends NativeActivity {
    static { System.loadLibrary("gallery"); } // so Java finds the native methods

    private ContentObserver observer;
    private static final int ASK_ACCESS = 1;

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // Bar sizes straight from Android: from Android 15 apps are drawn
        // under the bars, so the content area no longer shows them.
        getWindow().getDecorView().setOnApplyWindowInsetsListener((v, insets) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets i = insets.getInsets(
                        android.view.WindowInsets.Type.systemBars() | android.view.WindowInsets.Type.displayCutout());
                onInsets(i.top, i.bottom, i.left, i.right);
            } else {
                onInsets(insets.getStableInsetTop(), insets.getStableInsetBottom(),
                        insets.getStableInsetLeft(), insets.getStableInsetRight());
            }
            return v.onApplyWindowInsets(insets);
        });
        reportAccess();
    }

    @Override protected void onStart() {
        super.onStart();
        observer = new ContentObserver(new Handler(Looper.getMainLooper())) {
            @Override public void onChange(boolean selfChange) { onLibraryChanged(); }
            // Android files away the thumbnails we ask for and announces each
            // one; those are not changes to the library.
            @Override public void onChange(boolean selfChange, Uri uri) {
                if (uri != null && uri.getPath() != null && uri.getPath().contains("thumbnails")) return;
                onLibraryChanged();
            }
        };
        // The whole media store: Android 9 announces new pictures under
        // images/, not file/, so listening on the file list alone misses them.
        getContentResolver().registerContentObserver(Uri.parse("content://media"), true, observer);
        // Anything may have changed while we were away.
        onLibraryChanged();
    }

    @Override protected void onStop() {
        if (observer != null) getContentResolver().unregisterContentObserver(observer);
        observer = null;
        super.onStop();
    }

    @Override protected void onResume() {
        super.onResume();
        reportAccess(); // the owner may have changed it in Android's settings
    }

    private boolean granted(String p) {
        return checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED;
    }

    private void reportAccess() {
        int sdk = Build.VERSION.SDK_INT;
        boolean images = sdk >= 33 && granted("android.permission.READ_MEDIA_IMAGES");
        boolean video = sdk >= 33 && granted("android.permission.READ_MEDIA_VIDEO");
        boolean selected = sdk >= 34 && granted("android.permission.READ_MEDIA_VISUAL_USER_SELECTED");
        boolean legacy = sdk < 33 && granted(Manifest.permission.READ_EXTERNAL_STORAGE);
        onAccess(sdk, images, video, selected, legacy);
    }

    /** Rust: ask for the permissions that let us see the library. */
    public void requestAccess() {
        runOnUiThread(() -> {
            ArrayList<String> ps = new ArrayList<>();
            int sdk = Build.VERSION.SDK_INT;
            if (sdk >= 33) {
                ps.add("android.permission.READ_MEDIA_IMAGES");
                ps.add("android.permission.READ_MEDIA_VIDEO");
            } else {
                ps.add(Manifest.permission.READ_EXTERNAL_STORAGE);
                if (sdk <= 29) ps.add(Manifest.permission.WRITE_EXTERNAL_STORAGE);
            }
            if (sdk >= 29) ps.add(Manifest.permission.ACCESS_MEDIA_LOCATION);
            requestPermissions(ps.toArray(new String[0]), ASK_ACCESS);
        });
    }

    /** Rust: Android won't ask again; open our page in its settings. */
    public void openAppSettings() {
        runOnUiThread(() -> startActivity(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.parse("package:" + getPackageName()))));
    }

    /** Rust: whether Android would still show the permission question. */
    public boolean canAskAccess() {
        String p = Build.VERSION.SDK_INT >= 33 ? "android.permission.READ_MEDIA_IMAGES"
                : Manifest.permission.READ_EXTERNAL_STORAGE;
        return shouldShowRequestPermissionRationale(p);
    }

    @Override public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
        super.onRequestPermissionsResult(code, perms, results);
        onAsked();
        reportAccess();
        onLibraryChanged();
    }

    /** Rust (library thread): every photo and video, packed. */
    public byte[] queryLibrary(boolean trashed) { return Library.query(this, trashed); }

    /** Rust (thumbnail threads): one square thumbnail, or null. */
    public byte[] loadThumb(long id, boolean video, int orientation, int size) {
        return Thumbs.load(this, id, video, orientation, size);
    }

    /** Rust (viewer threads): a whole picture at screen size; see Pictures. */
    public byte[] decodePicture(long id, int orientation, int maxW, int maxH) {
        return Pictures.decode(this, id, orientation, maxW, maxH);
    }

    /** Rust (viewer threads): part of a picture at full detail. */
    public byte[] decodeRegion(long id, int orientation, int x, int y, int w, int h, int sample) {
        return Pictures.decodeRegion(this, id, orientation, x, y, w, h, sample);
    }

    /** Rust (viewer threads): a GIF's bytes. */
    public byte[] readAll(long id) { return Pictures.readAll(this, id); }

    /** Rust (INFO panel): camera, aperture, shutter, ISO, location, one per line. */
    public String pictureDetails(long id) { return String.join("\n", Pictures.details(this, id)); }

    static native void onInsets(int top, int bottom, int left, int right);
    static native void onAccess(int sdk, boolean images, boolean video, boolean selected, boolean legacy);
    static native void onAsked();
    static native void onLibraryChanged();
}
