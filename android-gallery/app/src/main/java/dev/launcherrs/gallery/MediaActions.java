package dev.launcherrs.gallery;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.media.MediaScannerConnection;
import android.media.ThumbnailUtils;
import android.net.Uri;
import android.os.Build;
import android.provider.MediaStore;

import java.io.File;
import java.util.ArrayList;
import java.util.List;

/** Share, Bin, restore, delete for good and favourites.
 *  Android 11+: Android's own Bin and confirm dialogs (none when the owner
 *  allowed "media management"). Android 8-10 have no Bin: deleted files
 *  move into the app's own folder; Rust keeps the list. */
final class MediaActions {
    private MediaActions() {}

    static final int TRASH = 1, RESTORE = 2, DELETE = 3, FAVOURITE = 4;

    static ArrayList<Uri> uris(long[] ids, boolean[] video) {
        ArrayList<Uri> out = new ArrayList<>();
        for (int i = 0; i < ids.length; i++) out.add(Thumbs.uri(ids[i], video[i]));
        return out;
    }

    static void share(Activity a, long[] ids, boolean[] video) {
        ArrayList<Uri> list = uris(ids, video);
        if (list.isEmpty()) return;
        boolean anyVideo = false, anyImage = false;
        for (boolean v : video) { if (v) anyVideo = true; else anyImage = true; }
        String type = anyVideo && anyImage ? "*/*" : anyVideo ? "video/*" : "image/*";
        Intent i;
        if (list.size() == 1) {
            i = new Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, list.get(0));
        } else {
            i = new Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, list);
        }
        i.setType(type);
        // The grant travels with the clip, so the receiving app may read every file.
        ClipData clip = ClipData.newRawUri("", list.get(0));
        for (int k = 1; k < list.size(); k++) clip.addItem(new ClipData.Item(list.get(k)));
        i.setClipData(clip);
        i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        Intent chooser = Intent.createChooser(i, null);
        a.runOnUiThread(() -> a.startActivity(chooser));
    }

    /** Android 11+: ask Android, which may show its confirm dialog. The
     *  answer comes to the activity's onActivityResult. */
    static void request(Activity a, int kind, long[] ids, boolean[] video, boolean on) {
        ContentResolver cr = a.getContentResolver();
        List<Uri> list = uris(ids, video);
        a.runOnUiThread(() -> {
            try {
                PendingIntent pi;
                switch (kind) {
                    case TRASH: pi = MediaStore.createTrashRequest(cr, list, true); break;
                    case RESTORE: pi = MediaStore.createTrashRequest(cr, list, false); break;
                    case DELETE: pi = MediaStore.createDeleteRequest(cr, list); break;
                    default: pi = MediaStore.createFavoriteRequest(cr, list, on);
                }
                a.startIntentSenderForResult(pi.getIntentSender(), 100 + kind, null, 0, 0, 0);
            } catch (Exception e) {
                MainActivity.onActionDone(kind, false);
            }
        });
    }

    /** The app's own bin folder (Android 8-10), kept out of other galleries. */
    static File binDir(Context c) {
        File d = new File(c.getExternalFilesDir(null), "bin");
        d.mkdirs();
        try { new File(d, ".nomedia").createNewFile(); } catch (Exception ignored) {}
        return d;
    }

    private static String pathOf(ContentResolver cr, Uri uri) {
        try (Cursor c = cr.query(uri, new String[] {MediaStore.MediaColumns.DATA}, null, null, null)) {
            return c != null && c.moveToFirst() ? c.getString(0) : null;
        } catch (Exception e) {
            return null;
        }
    }

    /** Android 8-10: move each file into the bin folder. Returns one line per
     *  file moved: "stored TAB original TAB deleted-ms" (Rust's bin format). */
    static String trashLegacy(Context c, long[] ids, boolean[] video) {
        ContentResolver cr = c.getContentResolver();
        File bin = binDir(c);
        long now = System.currentTimeMillis();
        StringBuilder out = new StringBuilder();
        ArrayList<String> gone = new ArrayList<>();
        for (int i = 0; i < ids.length; i++) {
            Uri uri = Thumbs.uri(ids[i], video[i]);
            String path = pathOf(cr, uri);
            if (path == null) continue;
            File from = new File(path);
            File to = new File(bin, now + "_" + i + "_" + from.getName());
            if (!from.renameTo(to)) continue;
            try { cr.delete(uri, null, null); } catch (Exception ignored) {}
            gone.add(path);
            out.append(to.getAbsolutePath()).append('\t').append(path).append('\t').append(now).append('\n');
        }
        if (!gone.isEmpty()) MediaScannerConnection.scanFile(c, gone.toArray(new String[0]), null, null);
        return out.toString();
    }

    /** Android 8-10: put a file back where it was; Android lists it again. */
    static boolean restoreLegacy(Context c, String stored, String original) {
        File from = new File(stored), to = new File(original);
        File parent = to.getParentFile();
        if (parent != null) parent.mkdirs();
        if (to.exists() || !from.renameTo(to)) return false;
        MediaScannerConnection.scanFile(c, new String[] {original}, null, null);
        return true;
    }

    /** A thumbnail for a file Android no longer lists (in our own bin). */
    static Bitmap thumbForPath(String path, boolean video) {
        try {
            if (video) return ThumbnailUtils.createVideoThumbnail(path, MediaStore.Images.Thumbnails.MINI_KIND);
            return ThumbnailUtils.createImageThumbnail(path, MediaStore.Images.Thumbnails.MINI_KIND);
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    static boolean canManageMedia(Context c) {
        return Build.VERSION.SDK_INT >= 31 && MediaStore.canManageMedia(c);
    }
}
