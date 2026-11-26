package dev.launcherrs.gallery;

import android.content.ContentResolver;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.MediaStore;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/** Reads every photo and video from Android's media list and packs them
 *  in the byte format Rust reads (gallery-core's media.rs). */
final class Library {
    private Library() {}

    private static final String[] COLUMNS = {
            "_id", "media_type", "mime_type", "width", "height", "orientation", "duration",
            "_size", "datetaken", "date_modified", "date_added", "bucket_id", "bucket_display_name",
            "_display_name"};

    static byte[] query(Context c, boolean trashed) {
        ContentResolver cr = c.getContentResolver();
        Uri uri = MediaStore.Files.getContentUri("external");
        String where = "media_type IN (1,3)";
        String[] cols = COLUMNS;
        boolean fav = Build.VERSION.SDK_INT >= 30;
        if (fav) {
            cols = java.util.Arrays.copyOf(COLUMNS, COLUMNS.length + 2);
            cols[COLUMNS.length] = "is_favorite";
            cols[COLUMNS.length + 1] = "date_expires";
        }
        Cursor cur;
        if (Build.VERSION.SDK_INT >= 30) {
            Bundle args = new Bundle();
            args.putString(android.content.ContentResolver.QUERY_ARG_SQL_SELECTION, where);
            args.putString(android.content.ContentResolver.QUERY_ARG_SQL_SORT_ORDER, "_id");
            args.putInt(MediaStore.QUERY_ARG_MATCH_TRASHED,
                    trashed ? MediaStore.MATCH_ONLY : MediaStore.MATCH_EXCLUDE);
            cur = cr.query(uri, cols, args, null);
        } else {
            if (trashed) return pack(0, new ByteArrayOutputStream());
            cur = cr.query(uri, cols, where, null, "_id");
        }
        ByteArrayOutputStream out = new ByteArrayOutputStream(64 * 1024);
        int count = 0;
        if (cur != null) {
            try {
                ByteBuffer rec = ByteBuffer.allocate(96).order(ByteOrder.LITTLE_ENDIAN);
                while (cur.moveToNext()) {
                    String mime = str(cur, 2);
                    int kind = cur.getInt(1) == 3 ? 1 : ("image/gif".equals(mime) ? 2 : 0);
                    rec.clear();
                    rec.putLong(cur.getLong(0));
                    rec.put((byte) kind);
                    rec.putInt(cur.getInt(3));
                    rec.putInt(cur.getInt(4));
                    rec.putShort((short) cur.getInt(5));
                    rec.putLong(cur.getLong(6));
                    rec.putLong(cur.getLong(7));
                    rec.putLong(cur.getLong(8));
                    rec.putLong(cur.getLong(9) * 1000L);
                    rec.putLong(cur.getLong(10) * 1000L);
                    rec.put((byte) (fav && cur.getInt(14) != 0 ? 1 : 0));
                    rec.putLong(cur.getLong(11));
                    rec.putLong(trashed && fav ? cur.getLong(15) * 1000L : 0L);
                    out.write(rec.array(), 0, rec.position());
                    text(out, str(cur, 13));
                    text(out, str(cur, 12));
                    text(out, mime);
                    count++;
                }
            } finally {
                cur.close();
            }
        }
        return pack(count, out);
    }

    private static String str(Cursor c, int i) {
        String s = c.getString(i);
        return s == null ? "" : s;
    }

    private static void text(ByteArrayOutputStream out, String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        int n = Math.min(b.length, 65535);
        out.write(n & 0xFF);
        out.write((n >> 8) & 0xFF);
        out.write(b, 0, n);
    }

    private static byte[] pack(int count, ByteArrayOutputStream body) {
        ByteBuffer head = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN);
        head.put((byte) 'H').put((byte) 'G').put((byte) 'L').put((byte) '2').putInt(count);
        byte[] b = body.toByteArray();
        byte[] all = new byte[8 + b.length];
        System.arraycopy(head.array(), 0, all, 0, 8);
        System.arraycopy(b, 0, all, 8, b.length);
        return all;
    }
}
