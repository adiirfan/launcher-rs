package dev.launcherrs.gallery;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.BitmapRegionDecoder;
import android.graphics.ImageDecoder;
import android.graphics.Rect;
import android.media.ExifInterface;
import android.net.Uri;
import android.os.Build;
import android.os.ParcelFileDescriptor;
import android.provider.MediaStore;

import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/** Reading pictures at full size for editing and writing the results.
 *  The pixels stay in Android's bitmaps; Rust works on them in place. */
final class Editing {
    private Editing() {}

    static final int JPEG = 0, PNG = 1, WEBP = 2, HEIC = 3;

    /** The original file, with its location kept when we're allowed to read it. */
    static Uri original(Context c, long id) {
        Uri uri = Thumbs.uri(id, false);
        if (id != Thumbs.FOREIGN && Build.VERSION.SDK_INT >= 29
                && c.checkSelfPermission(Manifest.permission.ACCESS_MEDIA_LOCATION) == PackageManager.PERMISSION_GRANTED) {
            uri = MediaStore.setRequireOriginal(uri);
        }
        return uri;
    }

    /** Part of the picture as stored (not turned upright), full detail. */
    static Bitmap region(Context c, long id, int x, int y, int w, int h) {
        try (InputStream in = c.getContentResolver().openInputStream(Thumbs.uri(id, false))) {
            BitmapRegionDecoder dec = Build.VERSION.SDK_INT >= 31
                    ? BitmapRegionDecoder.newInstance(in) : BitmapRegionDecoder.newInstance(in, false);
            if (dec == null) return null;
            try {
                Rect r = new Rect(x, y, x + w, y + h);
                if (!r.intersect(0, 0, dec.getWidth(), dec.getHeight())) return null;
                BitmapFactory.Options o = new BitmapFactory.Options();
                o.inPreferredConfig = Bitmap.Config.ARGB_8888;
                return dec.decodeRegion(r, o);
            } finally {
                dec.recycle();
            }
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    /** The whole picture, turned upright (for formats that can't be read in parts). */
    static Bitmap whole(Context c, long id) {
        try {
            if (Build.VERSION.SDK_INT >= 28) {
                ImageDecoder.Source src = ImageDecoder.createSource(c.getContentResolver(), Thumbs.uri(id, false));
                return ImageDecoder.decodeBitmap(src, (d, info, s) -> d.setAllocator(ImageDecoder.ALLOCATOR_SOFTWARE));
            }
            try (InputStream in = c.getContentResolver().openInputStream(Thumbs.uri(id, false))) {
                return BitmapFactory.decodeStream(in);
            }
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    static Bitmap blank(int w, int h) {
        try {
            return Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888);
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    /** Write `b` into the open file `fd` (which this closes), copying the
     *  original's date, camera and (unless `noLocation`) location for JPEGs. */
    static boolean write(Context c, long srcId, Bitmap b, int format, int quality, int fd, boolean keepDetails, boolean noLocation) {
        try (ParcelFileDescriptor pfd = ParcelFileDescriptor.adoptFd(fd)) {
            // JPEG and our HEIC have no see-through parts: white, not black.
            if ((format == JPEG || format == HEIC) && b.hasAlpha()) b = onWhite(b);
            if (format == HEIC) return writeHeic(pfd, b, quality);
            try (OutputStream out = new FileOutputStream(pfd.getFileDescriptor())) {
                Bitmap.CompressFormat f = format == PNG ? Bitmap.CompressFormat.PNG
                        : format == WEBP ? (Build.VERSION.SDK_INT >= 30 ? Bitmap.CompressFormat.WEBP_LOSSY : Bitmap.CompressFormat.WEBP)
                        : Bitmap.CompressFormat.JPEG;
                if (!b.compress(f, quality, out)) return false;
                out.flush();
            }
            if (format == JPEG && keepDetails) {
                copyDetails(c, srcId, pfd, b.getWidth(), b.getHeight(), noLocation);
            }
            return true;
        } catch (Exception | OutOfMemoryError e) {
            return false;
        }
    }

    /** See-through parts on white rather than black. */
    private static Bitmap onWhite(Bitmap b) {
        Bitmap w = Bitmap.createBitmap(b.getWidth(), b.getHeight(), Bitmap.Config.ARGB_8888);
        android.graphics.Canvas cv = new android.graphics.Canvas(w);
        cv.drawColor(android.graphics.Color.WHITE);
        cv.drawBitmap(b, 0, 0, null);
        return w;
    }

    private static boolean writeHeic(ParcelFileDescriptor pfd, Bitmap b, int quality) throws Exception {
        if (Build.VERSION.SDK_INT < 28) return false;
        androidx.heifwriter.HeifWriter w = new androidx.heifwriter.HeifWriter.Builder(pfd.getFileDescriptor(),
                b.getWidth(), b.getHeight(), androidx.heifwriter.HeifWriter.INPUT_MODE_BITMAP)
                .setQuality(quality).setMaxImages(1).build();
        try {
            w.start();
            w.addBitmap(b);
            w.stop(30_000);
        } finally {
            w.close();
        }
        return true;
    }

    /** Whether this phone can make HEIC pictures (it needs an HEVC encoder). */
    static boolean canHeic() {
        if (Build.VERSION.SDK_INT < 28) return false;
        for (android.media.MediaCodecInfo i : new android.media.MediaCodecList(android.media.MediaCodecList.REGULAR_CODECS).getCodecInfos()) {
            if (!i.isEncoder()) continue;
            for (String t : i.getSupportedTypes()) {
                if (t.equalsIgnoreCase("video/hevc") || t.equalsIgnoreCase("image/vnd.android.heic")) return true;
            }
        }
        return false;
    }

    /** The whole picture upright at `w` x `h`, for converting. */
    static Bitmap scaled(Context c, long id, int orientation, int w, int h) {
        try {
            if (Build.VERSION.SDK_INT >= 28) {
                ImageDecoder.Source src = ImageDecoder.createSource(c.getContentResolver(), Thumbs.uri(id, false));
                return ImageDecoder.decodeBitmap(src, (d, info, s) -> {
                    d.setAllocator(ImageDecoder.ALLOCATOR_SOFTWARE);
                    if (w > 0 && h > 0) d.setTargetSize(w, h);
                });
            }
            Bitmap b = whole(c, id);
            if (b == null) return null;
            if (orientation != 0) {
                android.graphics.Matrix m = new android.graphics.Matrix();
                m.postRotate(orientation);
                Bitmap r = Bitmap.createBitmap(b, 0, 0, b.getWidth(), b.getHeight(), m, true);
                if (r != b) b.recycle();
                b = r;
            }
            if (w > 0 && h > 0 && (b.getWidth() != w || b.getHeight() != h)) {
                Bitmap s = Bitmap.createScaledBitmap(b, w, h, true);
                if (s != b) b.recycle();
                b = s;
            }
            return b;
        } catch (Exception | OutOfMemoryError e) {
            android.util.Log.w("gallery", "convert: reading failed", e);
            return null;
        }
    }

    /** Convert one picture into the open file `fd` (closed here). */
    static boolean convert(Context c, long id, int orientation, int w, int h, int format, int quality, int fd, boolean keepDetails, boolean noLocation) {
        Bitmap b = scaled(c, id, orientation, w, h);
        if (b == null) {
            try { ParcelFileDescriptor.adoptFd(fd).close(); } catch (Exception ignored) {}
            return false;
        }
        try {
            return write(c, id, b, format, quality, fd, keepDetails, noLocation);
        } finally {
            b.recycle();
        }
    }

    private static final String[] KEEP = {
        ExifInterface.TAG_DATETIME, ExifInterface.TAG_DATETIME_ORIGINAL, ExifInterface.TAG_DATETIME_DIGITIZED,
        "OffsetTime", "OffsetTimeOriginal", "OffsetTimeDigitized",
        ExifInterface.TAG_MAKE, ExifInterface.TAG_MODEL, ExifInterface.TAG_F_NUMBER, ExifInterface.TAG_EXPOSURE_TIME,
        ExifInterface.TAG_ISO_SPEED_RATINGS, ExifInterface.TAG_FOCAL_LENGTH, "FocalLengthIn35mmFilm",
        ExifInterface.TAG_FLASH, ExifInterface.TAG_WHITE_BALANCE, ExifInterface.TAG_SUBSEC_TIME,
        "SubSecTimeOriginal", "SubSecTimeDigitized",
    };
    private static final String[] PLACE = {
        ExifInterface.TAG_GPS_LATITUDE, ExifInterface.TAG_GPS_LATITUDE_REF, ExifInterface.TAG_GPS_LONGITUDE,
        ExifInterface.TAG_GPS_LONGITUDE_REF, ExifInterface.TAG_GPS_ALTITUDE, ExifInterface.TAG_GPS_ALTITUDE_REF,
        ExifInterface.TAG_GPS_TIMESTAMP, ExifInterface.TAG_GPS_DATESTAMP,
    };

    private static void copyDetails(Context c, long srcId, ParcelFileDescriptor pfd, int w, int h, boolean noLocation) {
        try (InputStream in = c.getContentResolver().openInputStream(original(c, srcId))) {
            ExifInterface from = new ExifInterface(in);
            // Back to the start: the details reader looks at the file from here.
            android.system.Os.lseek(pfd.getFileDescriptor(), 0, android.system.OsConstants.SEEK_SET);
            ExifInterface to = new ExifInterface(pfd.getFileDescriptor());
            for (String t : KEEP) {
                String v = from.getAttribute(t);
                if (v != null) to.setAttribute(t, v);
            }
            // Without the media-location permission Android hands us zeros:
            // better no location than a wrong one.
            float[] ll = new float[2];
            boolean real = from.getLatLong(ll) && !(ll[0] == 0f && ll[1] == 0f)
                    && (Build.VERSION.SDK_INT < 29 || c.checkSelfPermission(Manifest.permission.ACCESS_MEDIA_LOCATION) == PackageManager.PERMISSION_GRANTED);
            if (!noLocation && real) {
                for (String t : PLACE) {
                    String v = from.getAttribute(t);
                    if (v != null) to.setAttribute(t, v);
                }
            }
            to.setAttribute(ExifInterface.TAG_ORIENTATION, "1");
            to.setAttribute(ExifInterface.TAG_IMAGE_WIDTH, Integer.toString(w));
            to.setAttribute(ExifInterface.TAG_IMAGE_LENGTH, Integer.toString(h));
            to.setAttribute(ExifInterface.TAG_SOFTWARE, "Holo Gallery");
            to.saveAttributes();
        } catch (Exception e) {
            // The picture is saved; only its details are missing.
            android.util.Log.w("gallery", "copying photo details failed", e);
        }
    }

    /** A JPEG only turned or flipped: copy the file and change its
     *  orientation tag, without touching the pixels. Closes `fd`. */
    static boolean losslessRotate(Context c, long id, int fd, int orientation) {
        try (ParcelFileDescriptor pfd = ParcelFileDescriptor.adoptFd(fd)) {
            try (InputStream in = c.getContentResolver().openInputStream(original(c, id));
                 OutputStream out = new FileOutputStream(pfd.getFileDescriptor())) {
                if (in == null) return false;
                byte[] buf = new byte[1 << 16];
                int n;
                while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
                out.flush();
            }
            android.system.Os.lseek(pfd.getFileDescriptor(), 0, android.system.OsConstants.SEEK_SET);
            ExifInterface e = new ExifInterface(pfd.getFileDescriptor());
            e.setAttribute(ExifInterface.TAG_ORIENTATION, Integer.toString(orientation));
            e.saveAttributes();
            return true;
        } catch (Exception e) {
            android.util.Log.w("gallery", "lossless rotate failed", e);
            return false;
        }
    }
}
