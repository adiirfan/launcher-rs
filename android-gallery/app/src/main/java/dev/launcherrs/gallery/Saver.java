package dev.launcherrs.gallery;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.media.MediaScannerConnection;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.os.ParcelFileDescriptor;
import android.provider.MediaStore;

import java.io.File;

/** New files for edited and converted copies. Android 11+ goes through the
 *  media list (the file stays hidden until it's finished); older versions
 *  write the file directly and ask Android to look at it afterwards. */
final class Saver {
    private Saver() {}

    private static boolean modern() { return Build.VERSION.SDK_INT >= 30; }

    private static File root() { return Environment.getExternalStorageDirectory(); }

    /** The folder an item is in, relative to the storage root, or "". */
    static String relPath(Context c, long id, boolean video) {
        Uri u = Thumbs.uri(id, video);
        String col = modern() ? MediaStore.MediaColumns.RELATIVE_PATH : MediaStore.MediaColumns.DATA;
        try (Cursor cur = c.getContentResolver().query(u, new String[] {col}, null, null, null)) {
            if (cur == null || !cur.moveToFirst() || cur.isNull(0)) return "";
            String v = cur.getString(0);
            if (modern()) return v;
            File parent = new File(v).getParentFile();
            String base = root().getAbsolutePath() + "/";
            String p = parent == null ? "" : parent.getAbsolutePath() + "/";
            return p.startsWith(base) ? p.substring(base.length()) : "";
        } catch (Exception e) {
            return "";
        }
    }

    /** Whether a file of that name is already in the folder. */
    static boolean exists(Context c, String relPath, String name) {
        if (!modern()) return new File(new File(root(), relPath), name).exists();
        String[] args = {relPath, name};
        try (Cursor cur = c.getContentResolver().query(MediaStore.Files.getContentUri("external"),
                new String[] {MediaStore.MediaColumns._ID},
                MediaStore.MediaColumns.RELATIVE_PATH + "=? AND " + MediaStore.MediaColumns.DISPLAY_NAME + "=?",
                args, null)) {
            return cur != null && cur.getCount() > 0;
        } catch (Exception e) {
            return false;
        }
    }

    /** Start a new, hidden file; returns its address, or null. */
    static String begin(Context c, String relPath, String name, String mime, long takenMs, boolean video) {
        try {
            if (modern()) {
                Uri coll = video ? MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
                        : MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY);
                ContentValues v = new ContentValues();
                v.put(MediaStore.MediaColumns.DISPLAY_NAME, name);
                v.put(MediaStore.MediaColumns.MIME_TYPE, mime);
                v.put(MediaStore.MediaColumns.RELATIVE_PATH, relPath);
                v.put(MediaStore.MediaColumns.IS_PENDING, 1);
                if (takenMs > 0) v.put(MediaStore.MediaColumns.DATE_TAKEN, takenMs);
                Uri u = c.getContentResolver().insert(coll, v);
                return u == null ? null : u.toString();
            }
            File dir = new File(root(), relPath);
            if (!dir.isDirectory() && !dir.mkdirs()) return null;
            File f = new File(dir, name);
            if (!f.createNewFile()) return null;
            return Uri.fromFile(f).toString();
        } catch (Exception e) {
            return null;
        }
    }

    /** A file descriptor for writing ("w") or reading ("r"); -1 on failure.
     *  The caller owns and closes it. */
    static int openFd(Context c, String uri, String mode) {
        try {
            Uri u = Uri.parse(uri);
            ParcelFileDescriptor pfd = "file".equals(u.getScheme())
                    ? ParcelFileDescriptor.open(new File(u.getPath()), mode.contains("w")
                            ? ParcelFileDescriptor.MODE_READ_WRITE | ParcelFileDescriptor.MODE_TRUNCATE
                            : ParcelFileDescriptor.MODE_READ_ONLY)
                    : c.getContentResolver().openFileDescriptor(u, mode.contains("w") ? "rwt" : "r");
            return pfd == null ? -1 : pfd.detachFd();
        } catch (Exception e) {
            return -1;
        }
    }

    /** The copy is complete: show it, with the original's date taken. */
    static boolean finish(Context c, String uri, long takenMs) {
        try {
            Uri u = Uri.parse(uri);
            if (modern()) {
                ContentValues v = new ContentValues();
                v.put(MediaStore.MediaColumns.IS_PENDING, 0);
                if (takenMs > 0) v.put(MediaStore.MediaColumns.DATE_TAKEN, takenMs);
                return c.getContentResolver().update(u, v, null, null) > 0;
            }
            File f = new File(u.getPath());
            if (takenMs > 0) f.setLastModified(takenMs);
            MediaScannerConnection.scanFile(c, new String[] {f.getAbsolutePath()}, null, null);
            return true;
        } catch (Exception e) {
            return false;
        }
    }

    /** Give up on a copy (it failed or was cancelled). */
    static void abandon(Context c, String uri) {
        try {
            Uri u = Uri.parse(uri);
            if ("file".equals(u.getScheme())) {
                new File(u.getPath()).delete();
            } else {
                ContentResolver cr = c.getContentResolver();
                cr.delete(u, null, null);
            }
        } catch (Exception ignored) {
        }
    }
}
