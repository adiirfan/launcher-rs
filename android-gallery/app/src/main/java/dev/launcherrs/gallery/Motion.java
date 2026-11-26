package dev.launcherrs.gallery;

import android.content.Context;
import android.os.ParcelFileDescriptor;

import java.io.FileInputStream;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;

/** The first and last `scan` bytes of a picture, one after the other (the
 *  whole file when it's small), for spotting motion photos. */
final class Motion {
    private Motion() {}

    static byte[] ends(Context c, long id, int scan) {
        try (ParcelFileDescriptor pfd = c.getContentResolver().openFileDescriptor(Thumbs.uri(id, false), "r");
             FileInputStream in = new FileInputStream(pfd.getFileDescriptor());
             FileChannel ch = in.getChannel()) {
            long size = ch.size();
            if (size <= 2L * scan) {
                ByteBuffer all = ByteBuffer.allocate((int) size);
                while (all.hasRemaining() && ch.read(all, all.position()) > 0) {}
                return all.array();
            }
            ByteBuffer b = ByteBuffer.allocate(2 * scan);
            b.limit(scan);
            while (b.hasRemaining() && ch.read(b, b.position()) > 0) {}
            b.limit(2 * scan);
            long from = size - scan;
            while (b.hasRemaining() && ch.read(b, from + b.position() - scan) > 0) {}
            return b.array();
        } catch (Exception e) {
            return null;
        }
    }
}
