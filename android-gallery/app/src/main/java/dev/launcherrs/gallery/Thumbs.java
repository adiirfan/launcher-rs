package dev.launcherrs.gallery;

import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Build;
import android.provider.MediaStore;
import android.util.Size;

import java.nio.ByteBuffer;

/** Square thumbnails from Android's own thumbnail cache. */
final class Thumbs {
    private Thumbs() {}

    /** The id Rust uses for a picture another app handed us. */
    static final long FOREIGN = Long.MIN_VALUE;
    /** That picture's link (set by MainActivity before telling Rust). */
    static volatile Uri foreign;

    static Uri uri(long id, boolean video) {
        if (id == FOREIGN && foreign != null) return foreign;
        return ContentUris.withAppendedId(video ? MediaStore.Video.Media.EXTERNAL_CONTENT_URI
                : MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id);
    }

    /** `size`×`size` RGBA (premultiplied, as Android keeps it), centre-cropped
     *  and turned upright; null when Android has none. */
    static byte[] load(Context c, long id, boolean video, int orientation, int size) {
        return load(c, id, null, video, orientation, size);
    }

    /** With `path` set: a file Android no longer lists (in our own bin). */
    static byte[] load(Context c, long id, String path, boolean video, int orientation, int size) {
        Bitmap b = null, out = null;
        try {
            ContentResolver cr = c.getContentResolver();
            if (path != null) {
                b = MediaActions.thumbForPath(path, video);
            } else if (Build.VERSION.SDK_INT >= 29) {
                // Android 10+ hands these back already upright.
                b = cr.loadThumbnail(uri(id, video), new Size(size, size), null);
                orientation = 0;
            } else if (video) {
                b = MediaStore.Video.Thumbnails.getThumbnail(cr, id, MediaStore.Video.Thumbnails.MINI_KIND, null);
                orientation = 0;
            } else {
                b = MediaStore.Images.Thumbnails.getThumbnail(cr, id, MediaStore.Images.Thumbnails.MINI_KIND, null);
            }
            if (b == null) return null;
            out = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888);
            Canvas canvas = new Canvas(out);
            int w = b.getWidth(), h = b.getHeight(), s = Math.min(w, h);
            Rect src = new Rect((w - s) / 2, (h - s) / 2, (w - s) / 2 + s, (h - s) / 2 + s);
            canvas.rotate(orientation, size / 2f, size / 2f);
            canvas.drawBitmap(b, src, new Rect(0, 0, size, size), new Paint(Paint.FILTER_BITMAP_FLAG));
            ByteBuffer buf = ByteBuffer.allocate(size * size * 4);
            out.copyPixelsToBuffer(buf);
            return buf.array();
        } catch (Exception | OutOfMemoryError e) {
            return null;
        } finally {
            if (b != null) b.recycle();
            if (out != null) out.recycle();
        }
    }
}
