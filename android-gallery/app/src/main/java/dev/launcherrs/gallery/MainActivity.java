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
    private Player player;
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
        // Android 13+: Back (gesture or button) comes here instead of as a
        // key press; Rust decides what it closes. Without this, Android 16
        // closes the whole app on a back swipe.
        if (Build.VERSION.SDK_INT >= 33) {
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, MainActivity::onBack);
        }
        Updater.onUpdate(Updater.IDLE, Updater.myVersion(this), 0);
        // A rebuilt activity has already shown this one.
        if (savedInstanceState == null) deliver(getIntent());
    }

    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        deliver(intent);
    }

    // Called from Rust (settings).
    public void checkForUpdate() { runOnUiThread(() -> Updater.check(this)); }
    public void installUpdate() { runOnUiThread(() -> Updater.install(this)); }

    /** Android 12+: the page where the owner lets us delete, restore and
     *  favourite without a confirm box each time. */
    /** Android 10+: keep the system's edge swipes (Back) off these areas,
     *  given as x, y, w, h groups in screen pixels; empty clears them. */
    public void setGestureExclusion(int[] r) {
        if (Build.VERSION.SDK_INT < 29) return;
        java.util.ArrayList<android.graphics.Rect> list = new java.util.ArrayList<>();
        for (int i = 0; i + 3 < r.length; i += 4) list.add(new android.graphics.Rect(r[i], r[i + 1], r[i] + r[i + 2], r[i + 1] + r[i + 3]));
        runOnUiThread(() -> getWindow().getDecorView().setSystemGestureExclusionRects(list));
    }

    public void openManageMedia() {
        if (Build.VERSION.SDK_INT < 31) return;
        try {
            startActivity(new Intent(android.provider.Settings.ACTION_REQUEST_MANAGE_MEDIA,
                    Uri.parse("package:" + getPackageName())));
        } catch (Exception e) { /* no such page on this phone */ }
    }

    /** "Open with" from another app: hand the link to Rust. */
    private void deliver(Intent i) {
        if (i == null || i.getData() == null) return;
        String a = i.getAction();
        if (!Intent.ACTION_VIEW.equals(a) && !"android.provider.action.REVIEW".equals(a)
                && !"android.provider.action.REVIEW_SECURE".equals(a)) return;
        Uri u = i.getData();
        String mime = i.getType();
        if (mime == null) {
            try { mime = getContentResolver().getType(u); } catch (Exception e) { mime = null; }
        }
        if (mime == null) {
            String ext = android.webkit.MimeTypeMap.getFileExtensionFromUrl(u.toString());
            mime = ext == null ? null : android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase());
        }
        // Its name, size and date, where the other app says.
        String name = u.getLastPathSegment();
        long size = 0, modified = 0;
        try (android.database.Cursor c = getContentResolver().query(u, null, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                int n = c.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME);
                int s = c.getColumnIndex(android.provider.OpenableColumns.SIZE);
                int m = c.getColumnIndex("last_modified");
                if (n >= 0 && !c.isNull(n)) name = c.getString(n);
                if (s >= 0 && !c.isNull(s)) size = c.getLong(s);
                if (m >= 0 && !c.isNull(m)) modified = c.getLong(m);
            }
        } catch (Exception e) { /* a plain file link: no answers */ }
        if (name != null && name.contains("/")) name = name.substring(name.lastIndexOf('/') + 1);
        Thumbs.foreign = u;
        onOpen(u.toString(), mime == null ? "" : mime, name == null ? "" : name, size, modified);
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

    /** Going to the background pauses a video; it keeps its place. */
    @Override protected void onPause() {
        if (player != null) player.pause();
        super.onPause();
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

    private Player player() {
        if (player == null) player = new Player(this);
        return player;
    }

    /** Rust (drawing thread): the video player. See Player. */
    public void videoOpen(long id, int texName) { player().open(id, texName); }
    public float[] videoUpdate() { return player().update(); }
    public void videoPlay() { player().play(); }
    public void videoPause() { player().pause(); }
    public void videoSeek(int ms) { player().seek(ms); }
    public int videoPosition() { return player().position(); }
    public int videoDuration() { return player().duration(); }
    public boolean videoPlaying() { return player().playing(); }
    public void videoMute(boolean muted) { player().setMuted(muted); }
    public void videoClose() { player().close(); }

    /** Rust: share, Bin, favourites. See MediaActions. On Android 11+ the
     *  answer comes later through onActionDone. */
    public void share(long[] ids, boolean[] video) { MediaActions.share(this, ids, video); }
    public void mediaRequest(int kind, long[] ids, boolean[] video, boolean on) {
        MediaActions.request(this, kind, ids, video, on);
    }
    public boolean canHeic() { return Editing.canHeic(); }
    public boolean editConvert(long id, int orientation, int w, int h, int format, int quality, int fd, boolean keepDetails, boolean noLocation) {
        return Editing.convert(this, id, orientation, w, h, format, quality, fd, keepDetails, noLocation);
    }
    public void gifDone() { FrameReader.close(); }
    public boolean readerOpen(long id, long startUs) { return FrameReader.open(this, id, startUs); }
    public boolean readerAdvance(long us) { return FrameReader.advance(us); }
    public byte[] readerPlane(int i) { return FrameReader.plane(i); }
    public int[] readerInfo() { return FrameReader.info(); }
    public boolean gifVideoStart(int fd, int w, int h, int fps, int bitRate) { return GifVideo.start(fd, w, h, fps, bitRate); }
    public boolean gifVideoPut(byte[] yuv, long ptsUs) { return GifVideo.put(yuv, ptsUs); }
    public String gifVideoFinish() { return GifVideo.finish(); }
    public void gifVideoAbort() { GifVideo.abort(); }
    public void clipStrip(long id, int count, int height, int gen) { Frames.strip(this, id, count, height, gen); }
    public String clipRun(long id, long startUs, long endUs, boolean exact, boolean keepSound, int fd) {
        return Clipper.run(this, id, startUs, endUs, exact, keepSound, fd);
    }
    public boolean moveOne(long id, boolean video, String rel, String name) { return MediaActions.moveOne(this, id, video, rel, name); }
    public String trashLegacy(long[] ids, boolean[] video) { return MediaActions.trashLegacy(this, ids, video); }
    public boolean restoreLegacy(String stored, String original) { return MediaActions.restoreLegacy(this, stored, original); }
    public String binDir() { return MediaActions.binDir(this).getAbsolutePath(); }
    public byte[] loadThumbPath(String path, boolean video, int size) { return Thumbs.load(this, 0, path, video, 0, size); }
    public boolean canManageMedia() { return MediaActions.canManageMedia(this); }

    // Saving copies (Saver.java); called from Rust worker threads.
    public String saverRelPath(long id, boolean video) { return Saver.relPath(this, id, video); }
    public boolean saverExists(String rel, String name) { return Saver.exists(this, rel, name); }
    public String saverBegin(String rel, String name, String mime, long takenMs, boolean video) {
        return Saver.begin(this, rel, name, mime, takenMs, video);
    }
    public int saverOpenFd(String uri, String mode) { return Saver.openFd(this, uri, mode); }
    public boolean saverFinish(String uri, long takenMs) { return Saver.finish(this, uri, takenMs); }
    public void saverAbandon(String uri) { Saver.abandon(this, uri); }

    // Editing pictures (Editing.java); called from Rust worker threads.
    public android.graphics.Bitmap editRegion(long id, int x, int y, int w, int h) { return Editing.region(this, id, x, y, w, h); }
    public android.graphics.Bitmap editWhole(long id) { return Editing.whole(this, id); }
    public android.graphics.Bitmap editBlank(int w, int h) { return Editing.blank(w, h); }
    public boolean editWrite(long srcId, android.graphics.Bitmap b, int format, int quality, int fd, boolean keepDetails, boolean noLocation) {
        return Editing.write(this, srcId, b, format, quality, fd, keepDetails, noLocation);
    }
    public boolean editLossless(long id, int fd, int orientation) { return Editing.losslessRotate(this, id, fd, orientation); }
    public int sdk() { return Build.VERSION.SDK_INT; }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request > 100 && request <= 105) onActionDone(request - 100, result == RESULT_OK);
    }

    static native void onActionDone(int kind, boolean ok);
    static native void onBack();
    static native void onOpen(String uri, String mime, String name, long size, long modifiedMs);
    static native void onVideoFrame();
    static native void onStripFrame(int gen, int index, int w, int h, int[] argb);
    static native void onClipProgress(int percent);
    static native void onVideoState(int state, int w, int h);
    static native void onInsets(int top, int bottom, int left, int right);
    static native void onAccess(int sdk, boolean images, boolean video, boolean selected, boolean legacy);
    static native void onAsked();
    static native void onLibraryChanged();
}
