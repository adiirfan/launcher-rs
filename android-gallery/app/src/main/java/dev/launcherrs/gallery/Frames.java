package dev.launcherrs.gallery;

import android.content.Context;
import android.graphics.Bitmap;
import android.media.MediaMetadataRetriever;
import android.os.Build;
import android.util.Log;

/** Small frames from across a video, for the clip screen's strip: square, `size` pixels. They
 *  arrive one by one through `MainActivity.onStripFrame`. */
final class Frames {
    private Frames() {}

    static void strip(Context c, long id, int count, int size, int gen) {
        Thread t = new Thread(() -> {
            MediaMetadataRetriever r = new MediaMetadataRetriever();
            try {
                r.setDataSource(c, Thumbs.uri(id, true));
                long durMs = Long.parseLong(nonNull(r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)));
                int w = Integer.parseInt(nonNull(r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)));
                int h = Integer.parseInt(nonNull(r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)));
                String rot = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION);
                if (rot != null && (rot.equals("90") || rot.equals("270"))) { int s = w; w = h; h = s; }
                // Each frame: its middle square, `size` pixels.
                float scale = size / (float) Math.max(1, Math.min(w, h));
                int tw = Math.max(size, Math.round(w * scale)), th = Math.max(size, Math.round(h * scale));
                for (int i = 0; i < count; i++) {
                    long us = durMs * 1000L * (2L * i + 1) / (2L * count);
                    Bitmap b = Build.VERSION.SDK_INT >= 27
                            ? r.getScaledFrameAtTime(us, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, tw, th)
                            : r.getFrameAtTime(us, MediaMetadataRetriever.OPTION_CLOSEST_SYNC);
                    if (b == null) continue;
                    if (b.getWidth() != tw || b.getHeight() != th) {
                        Bitmap s = Bitmap.createScaledBitmap(b, tw, th, true);
                        if (s != b) b.recycle();
                        b = s;
                    }
                    Bitmap sq = Bitmap.createBitmap(b, (tw - size) / 2, (th - size) / 2, size, size);
                    if (sq != b) b.recycle();
                    b = sq;
                    int bw = b.getWidth(), bh = b.getHeight();
                    int[] px = new int[bw * bh];
                    b.getPixels(px, 0, bw, 0, 0, bw, bh);
                    b.recycle();
                    MainActivity.onStripFrame(gen, i, bw, bh, px);
                }
            } catch (Exception | OutOfMemoryError e) {
                Log.w("gallery", "strip frames failed", e);
            } finally {
                try { r.release(); } catch (Exception ignored) {}
            }
        }, "strip");
        t.start();
    }

    private static String nonNull(String s) {
        if (s == null) throw new IllegalStateException("missing video detail");
        return s;
    }
}
