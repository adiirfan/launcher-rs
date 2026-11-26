package dev.launcherrs.gallery;

import android.media.Image;
import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaFormat;
import android.media.MediaMuxer;
import android.os.ParcelFileDescriptor;
import android.util.Log;

import java.nio.ByteBuffer;

/** Saving a GIF as an MP4 video. Rust decodes the GIF and hands over each
 *  frame as planar YUV (all Y, then U, then V) with its time; this feeds the
 *  phone's H.264 encoder and writes the file. One at a time. */
final class GifVideo {
    private GifVideo() {}

    private static final long WAIT_US = 10_000;
    private static final String FAILED = "Couldn't make the video";

    private static ParcelFileDescriptor out;
    private static MediaMuxer mux;
    private static MediaCodec enc;
    private static int w, h, track = -1, frames;
    private static boolean started;
    private static byte[] last;
    private static final MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();

    /** Ready to write a `width` x `height` video into `fd` (closed by finish). */
    static synchronized boolean start(int fd, int width, int height, int fps, int bitRate) {
        abort();
        try {
            out = ParcelFileDescriptor.adoptFd(fd);
            mux = new MediaMuxer(out.getFileDescriptor(), MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4);
            MediaFormat f = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height);
            f.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible);
            f.setInteger(MediaFormat.KEY_BIT_RATE, bitRate);
            f.setInteger(MediaFormat.KEY_FRAME_RATE, Math.max(1, fps));
            f.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2);
            enc = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC);
            MediaCodecInfo.VideoCapabilities caps = enc.getCodecInfo().getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC).getVideoCapabilities();
            if (caps != null && !caps.isSizeSupported(width, height)) {
                Log.i("gallery", "gif video: encoder can't do " + width + "x" + height + " (widths " + caps.getSupportedWidths()
                        + " step " + caps.getWidthAlignment() + ", heights " + caps.getSupportedHeights() + " step " + caps.getHeightAlignment() + ")");
                abort();
                return false;
            }
            enc.configure(f, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE);
            enc.start();
            w = width;
            h = height;
            track = -1;
            frames = 0;
            started = false;
            last = null;
            return true;
        } catch (Exception e) {
            Log.w("gallery", "gif video: start failed at " + width + "x" + height, e);
            abort();
            return false;
        }
    }

    /** One frame at `ptsUs`; null `yuv` shows the previous one again. */
    static synchronized boolean put(byte[] yuv, long ptsUs) {
        if (enc == null) return false;
        if (yuv != null) last = yuv;
        if (last == null) return false;
        long give = System.nanoTime() + 10_000_000_000L;
        try {
            while (true) {
                int ii = enc.dequeueInputBuffer(WAIT_US);
                if (ii >= 0) {
                    Image img = enc.getInputImage(ii);
                    if (img == null) {
                        Log.w("gallery", "gif video: no input picture");
                        return false;
                    }
                    fill(img, last);
                    enc.queueInputBuffer(ii, 0, w * h * 3 / 2, ptsUs, 0);
                    frames++;
                    drain(false);
                    return true;
                }
                drain(false);
                if (System.nanoTime() > give) {
                    Log.w("gallery", "gif video: encoder stopped taking frames");
                    return false;
                }
            }
        } catch (Exception e) {
            Log.w("gallery", "gif video: frame failed", e);
            return false;
        }
    }

    /** Copy planar YUV into the encoder's picture, whatever its layout. */
    private static void fill(Image img, byte[] yuv) {
        Image.Plane[] p = img.getPlanes();
        int[] off = {0, w * h, w * h + w * h / 4};
        for (int i = 0; i < 3; i++) {
            int pw = i == 0 ? w : w / 2, ph = i == 0 ? h : h / 2;
            ByteBuffer b = p[i].getBuffer();
            int row = p[i].getRowStride(), pix = p[i].getPixelStride();
            for (int y = 0; y < ph; y++) {
                int src = off[i] + y * pw, dst = y * row;
                if (pix == 1) {
                    b.position(dst);
                    b.put(yuv, src, pw);
                } else {
                    for (int x = 0; x < pw; x++) b.put(dst + x * pix, yuv[src + x]);
                }
            }
        }
    }

    /** Write what the encoder has made; with `end`, everything until it's done. */
    private static void drain(boolean end) {
        long give = System.nanoTime() + 10_000_000_000L;
        while (true) {
            int oi = enc.dequeueOutputBuffer(info, end ? WAIT_US : 0);
            if (oi == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                track = mux.addTrack(enc.getOutputFormat());
                mux.start();
                started = true;
            } else if (oi >= 0) {
                ByteBuffer ob = enc.getOutputBuffer(oi);
                if ((info.flags & MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0) info.size = 0;
                if (info.size > 0 && started && ob != null) mux.writeSampleData(track, ob, info);
                enc.releaseOutputBuffer(oi, false);
                if ((info.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) return;
            } else if (!end) {
                return;
            } else if (System.nanoTime() > give) {
                Log.w("gallery", "gif video: gave up waiting for the end");
                return;
            }
        }
    }

    /** Finish the file. Null when it worked, otherwise a message. */
    static synchronized String finish() {
        if (enc == null) return FAILED;
        try {
            int ii = enc.dequeueInputBuffer(1_000_000);
            if (ii < 0) return FAILED;
            enc.queueInputBuffer(ii, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM);
            drain(true);
            if (!started || frames == 0) return FAILED;
            mux.stop();
            Log.i("gallery", "gif video: " + frames + " frames, " + w + "x" + h);
            return null;
        } catch (Exception e) {
            Log.w("gallery", "gif video: finish failed", e);
            return FAILED;
        } finally {
            abort();
        }
    }

    /** Let go of everything; the file is left as it is. */
    static synchronized void abort() {
        if (enc != null) {
            try { enc.stop(); } catch (Exception ignored) {}
            enc.release();
            enc = null;
        }
        if (mux != null) {
            try { mux.release(); } catch (Exception ignored) {}
            mux = null;
        }
        if (out != null) {
            try { out.close(); } catch (Exception ignored) {}
            out = null;
        }
        last = null;
    }
}
