package dev.launcherrs.gallery;

import android.Manifest;
import android.content.ContentResolver;
import android.content.Context;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.BitmapRegionDecoder;
import android.graphics.ImageDecoder;
import android.graphics.Matrix;
import android.graphics.Rect;
import android.media.ExifInterface;
import android.net.Uri;
import android.os.Build;
import android.provider.MediaStore;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/** Full pictures for the viewer. Every picture comes back as a 16-byte
 *  header (width, height, full width, full height; little-endian ints,
 *  upright) followed by premultiplied RGBA. */
final class Pictures {
    private Pictures() {}

    /** Biggest GIF read into memory whole. */
    private static final int MAX_RAW = 64 << 20;

    private static byte[] pack(Bitmap b, int fullW, int fullH) {
        if (b.getConfig() != Bitmap.Config.ARGB_8888) {
            Bitmap c = b.copy(Bitmap.Config.ARGB_8888, false);
            b.recycle();
            b = c;
        }
        int w = b.getWidth(), h = b.getHeight();
        ByteBuffer buf = ByteBuffer.allocate(16 + w * h * 4).order(ByteOrder.LITTLE_ENDIAN);
        buf.putInt(w).putInt(h).putInt(fullW).putInt(fullH);
        b.copyPixelsToBuffer(buf);
        b.recycle();
        return buf.array();
    }

    /** The scale that fits `w`×`h` inside a box whose long side is `maxLong`
     *  and short side `maxShort`, whichever way round the picture is. */
    private static float fitScale(int w, int h, int maxW, int maxH) {
        int maxLong = Math.max(maxW, maxH), maxShort = Math.min(maxW, maxH);
        float s = Math.min(maxLong / (float) Math.max(w, h), maxShort / (float) Math.min(w, h));
        return Math.min(1f, s);
    }

    /** The whole picture, upright, no bigger than `maxW`×`maxH` (either way
     *  round); null if Android can't read it. */
    static byte[] decode(Context c, long id, int orientation, int maxW, int maxH) {
        Uri uri = Thumbs.uri(id, false);
        try {
            if (Build.VERSION.SDK_INT >= 28) {
                final int[] raw = new int[2];
                ImageDecoder.Source src = ImageDecoder.createSource(c.getContentResolver(), uri);
                Bitmap b = ImageDecoder.decodeBitmap(src, (decoder, info, s) -> {
                    int w = info.getSize().getWidth(), h = info.getSize().getHeight();
                    raw[0] = w;
                    raw[1] = h;
                    float k = fitScale(w, h, maxW, maxH);
                    decoder.setTargetSize(Math.max(1, Math.round(w * k)), Math.max(1, Math.round(h * k)));
                    decoder.setAllocator(ImageDecoder.ALLOCATOR_SOFTWARE);
                });
                // ImageDecoder turns the picture upright itself; the full size
                // is the raw one, swapped if turning swapped the sides.
                boolean swapped = (b.getWidth() > b.getHeight()) != (raw[0] > raw[1]) && raw[0] != raw[1];
                return pack(b, swapped ? raw[1] : raw[0], swapped ? raw[0] : raw[1]);
            }
            ContentResolver cr = c.getContentResolver();
            BitmapFactory.Options o = new BitmapFactory.Options();
            o.inJustDecodeBounds = true;
            try (InputStream in = cr.openInputStream(uri)) { BitmapFactory.decodeStream(in, null, o); }
            int w = o.outWidth, h = o.outHeight;
            if (w <= 0 || h <= 0) return null;
            float k = fitScale(w, h, maxW, maxH);
            int sample = 1;
            while (w / (sample * 2) >= w * k && h / (sample * 2) >= h * k) sample *= 2;
            BitmapFactory.Options d = new BitmapFactory.Options();
            d.inSampleSize = sample;
            Bitmap b;
            try (InputStream in = cr.openInputStream(uri)) { b = BitmapFactory.decodeStream(in, null, d); }
            if (b == null) return null;
            b = turn(b, orientation);
            boolean sideways = orientation == 90 || orientation == 270;
            return pack(b, sideways ? h : w, sideways ? w : h);
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    private static Bitmap turn(Bitmap b, int orientation) {
        if (orientation == 0) return b;
        Matrix m = new Matrix();
        m.postRotate(orientation);
        Bitmap r = Bitmap.createBitmap(b, 0, 0, b.getWidth(), b.getHeight(), m, true);
        if (r != b) b.recycle();
        return r;
    }

    /** One part of the picture at full detail (`sample` 1) or halved
     *  (`sample` 2, 4...). `x, y, w, h` are in the upright picture's pixels.
     *  Null for formats Android can't decode in parts. */
    static byte[] decodeRegion(Context c, long id, int orientation, int x, int y, int w, int h, int sample) {
        try (InputStream in = c.getContentResolver().openInputStream(Thumbs.uri(id, false))) {
            BitmapRegionDecoder dec = Build.VERSION.SDK_INT >= 31
                    ? BitmapRegionDecoder.newInstance(in) : BitmapRegionDecoder.newInstance(in, false);
            if (dec == null) return null;
            try {
                int rw = dec.getWidth(), rh = dec.getHeight();
                Rect r;
                switch (orientation) {
                    case 90: r = new Rect(y, rh - (x + w), y + h, rh - x); break;
                    case 180: r = new Rect(rw - (x + w), rh - (y + h), rw - x, rh - y); break;
                    case 270: r = new Rect(rw - (y + h), x, rw - y, x + w); break;
                    default: r = new Rect(x, y, x + w, y + h);
                }
                if (!r.intersect(0, 0, rw, rh)) return null;
                BitmapFactory.Options o = new BitmapFactory.Options();
                o.inSampleSize = Math.max(1, sample);
                Bitmap b = dec.decodeRegion(r, o);
                if (b == null) return null;
                boolean sideways = orientation == 90 || orientation == 270;
                return pack(turn(b, orientation), sideways ? rh : rw, sideways ? rw : rh);
            } finally {
                dec.recycle();
            }
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    /** The file's bytes (for GIFs, decoded in Rust); null if too big or unreadable. */
    static byte[] readAll(Context c, long id) {
        try (InputStream in = c.getContentResolver().openInputStream(Thumbs.uri(id, false))) {
            if (in == null) return null;
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] chunk = new byte[1 << 16];
            int n;
            while ((n = in.read(chunk)) > 0) {
                out.write(chunk, 0, n);
                if (out.size() > MAX_RAW) return null;
            }
            return out.toByteArray();
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    /** For the INFO panel: camera, aperture, shutter, ISO, "lat,lon".
     *  Empty strings for anything the file doesn't say. */
    static String[] details(Context c, long id) {
        String[] out = {"", "", "", "", ""};
        Uri uri = Thumbs.uri(id, false);
        // The location is blanked out unless we ask for the original file,
        // which needs the media-location permission.
        if (Build.VERSION.SDK_INT >= 29
                && c.checkSelfPermission(Manifest.permission.ACCESS_MEDIA_LOCATION) == PackageManager.PERMISSION_GRANTED) {
            uri = MediaStore.setRequireOriginal(uri);
        }
        try (InputStream in = c.getContentResolver().openInputStream(uri)) {
            if (in == null) return out;
            ExifInterface e = new ExifInterface(in);
            String make = e.getAttribute(ExifInterface.TAG_MAKE), model = e.getAttribute(ExifInterface.TAG_MODEL);
            if (model != null) {
                out[0] = make != null && !model.toLowerCase().startsWith(make.toLowerCase().trim()) ? make.trim() + " " + model.trim() : model.trim();
            }
            double f = e.getAttributeDouble(ExifInterface.TAG_F_NUMBER, 0);
            if (f > 0) out[1] = String.format(java.util.Locale.ROOT, "f/%.1f", f);
            double t = e.getAttributeDouble(ExifInterface.TAG_EXPOSURE_TIME, 0);
            if (t > 0) out[2] = t >= 1 ? String.format(java.util.Locale.ROOT, "%.1f s", t) : "1/" + Math.round(1 / t) + " s";
            String iso = e.getAttribute(ExifInterface.TAG_ISO_SPEED_RATINGS);
            if (iso != null) out[3] = "ISO " + iso;
            float[] ll = new float[2];
            if (e.getLatLong(ll)) out[4] = ll[0] + "," + ll[1];
        } catch (Exception e) {
            // A file without these details: show what we have.
        }
        return out;
    }
}
