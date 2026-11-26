package dev.launcherrs.gallery;

import android.content.Context;
import android.graphics.Rect;
import android.media.Image;
import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import android.media.MediaMetadataRetriever;
import android.util.Log;

import java.nio.ByteBuffer;

/** Reads a video's frames in order with the phone's decoder, for making a
 *  GIF. Much faster than asking for each moment separately, which decodes
 *  from the last key frame every time. One at a time; Rust turns the
 *  frame's planes into an upright picture. */
final class FrameReader {
    private FrameReader() {}

    private static final long WAIT_US = 10_000;
    private static MediaExtractor ex;
    private static MediaCodec dec;
    private static boolean inDone, outDone;
    private static int rotation;

    // The frame we have: its time and planes.
    private static long pts = -1;
    private static final byte[][] planes = new byte[3][];
    /** w, h, left, top, yRow, uvRow, uvPix, rotation */
    private static final int[] info = new int[8];

    static synchronized boolean open(Context c, long id, long startUs) {
        close();
        try {
            MediaMetadataRetriever r = new MediaMetadataRetriever();
            try {
                r.setDataSource(c, Thumbs.uri(id, true));
                String rot = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION);
                rotation = rot == null ? 0 : Integer.parseInt(rot);
            } finally {
                r.release();
            }
            ex = new MediaExtractor();
            ex.setDataSource(c, Thumbs.uri(id, true), null);
            int v = -1;
            for (int i = 0; i < ex.getTrackCount(); i++) {
                String m = ex.getTrackFormat(i).getString(MediaFormat.KEY_MIME);
                if (m != null && m.startsWith("video/")) { v = i; break; }
            }
            if (v < 0) { close(); return false; }
            MediaFormat f = ex.getTrackFormat(v);
            ex.selectTrack(v);
            ex.seekTo(startUs, MediaExtractor.SEEK_TO_PREVIOUS_SYNC);
            f.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible);
            dec = MediaCodec.createDecoderByType(f.getString(MediaFormat.KEY_MIME));
            dec.configure(f, null, null, 0);
            dec.start();
            inDone = outDone = false;
            pts = -1;
            return true;
        } catch (Exception e) {
            Log.w("gallery", "frame reader: open failed", e);
            close();
            return false;
        }
    }

    /** Move on to the frame showing at `us` (the first one at or after it;
     *  the last one at the end of the video). False if there's none. */
    static synchronized boolean advance(long us) {
        if (dec == null) return false;
        if (pts >= us) return true;
        MediaCodec.BufferInfo bi = new MediaCodec.BufferInfo();
        long give = System.nanoTime() + 10_000_000_000L;
        try {
            while (!outDone) {
                if (System.nanoTime() > give) {
                    Log.w("gallery", "frame reader: gave up waiting at " + us + " us");
                    return pts >= 0;
                }
                if (!inDone) {
                    int ii = dec.dequeueInputBuffer(WAIT_US);
                    if (ii >= 0) {
                        ByteBuffer b = dec.getInputBuffer(ii);
                        int n = b == null ? -1 : ex.readSampleData(b, 0);
                        if (n < 0) {
                            dec.queueInputBuffer(ii, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM);
                            inDone = true;
                        } else {
                            dec.queueInputBuffer(ii, 0, n, ex.getSampleTime(), 0);
                            ex.advance();
                        }
                    }
                }
                int oi = dec.dequeueOutputBuffer(bi, WAIT_US);
                if (oi < 0) continue;
                if ((bi.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) outDone = true;
                boolean take = bi.size > 0 && (bi.presentationTimeUs >= us || pts < 0 && outDone);
                if (bi.size > 0 && (take || pts < 0)) {
                    // Keep this one: it's the target, or the best we have so far.
                    Image img = dec.getOutputImage(oi);
                    if (img != null) {
                        keep(img);
                        img.close();
                        pts = bi.presentationTimeUs;
                    }
                }
                dec.releaseOutputBuffer(oi, false);
                if (bi.size > 0 && bi.presentationTimeUs >= us) return true;
            }
            return pts >= 0;
        } catch (Exception e) {
            Log.w("gallery", "frame reader: decode failed", e);
            return pts >= 0;
        }
    }

    private static void keep(Image img) {
        Rect crop = img.getCropRect();
        Image.Plane[] p = img.getPlanes();
        for (int i = 0; i < 3; i++) {
            ByteBuffer b = p[i].getBuffer();
            b.rewind();
            if (planes[i] == null || planes[i].length != b.remaining()) planes[i] = new byte[b.remaining()];
            b.get(planes[i]);
        }
        info[0] = crop.width();
        info[1] = crop.height();
        info[2] = crop.left;
        info[3] = crop.top;
        info[4] = p[0].getRowStride();
        info[5] = p[1].getRowStride();
        info[6] = p[1].getPixelStride();
        info[7] = rotation;
    }

    static synchronized byte[] plane(int i) { return planes[i]; }

    static synchronized int[] info() { return info.clone(); }

    static synchronized void close() {
        if (dec != null) {
            try { dec.stop(); } catch (Exception ignored) {}
            dec.release();
            dec = null;
        }
        if (ex != null) {
            ex.release();
            ex = null;
        }
        planes[0] = planes[1] = planes[2] = null;
        pts = -1;
    }
}
