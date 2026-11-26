package dev.launcherrs.gallery;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import android.media.MediaMetadataRetriever;
import android.media.MediaMuxer;
import android.net.Uri;
import android.os.Build;
import android.os.ParcelFileDescriptor;
import android.provider.MediaStore;
import android.util.Log;
import android.view.Surface;

import java.nio.ByteBuffer;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Cuts a part out of a video into a new file, with the phone's own video
 *  chip. Instant copies the stored frames from the key frame at or before
 *  the start (no quality loss, may start early). Exact decodes and encodes
 *  the video again so it starts right at the handle; the sound is copied
 *  as it is either way. */
final class Clipper {
    private Clipper() {}

    static final String CANT_EXACT = "This video can't be re-saved exactly; use Instant";
    private static final String FAILED = "Couldn't save the clip";
    private static final long WAIT_US = 10_000;

    /** The video with its location kept when we're allowed to read it. */
    private static Uri original(Context c, long id) {
        Uri uri = Thumbs.uri(id, true);
        if (Build.VERSION.SDK_INT >= 29
                && c.checkSelfPermission(Manifest.permission.ACCESS_MEDIA_LOCATION) == PackageManager.PERMISSION_GRANTED) {
            uri = MediaStore.setRequireOriginal(uri);
        }
        return uri;
    }

    /** Write the clip into `fd` (closed here). Returns null when it worked,
     *  otherwise a message for the owner. Runs on the caller's thread. */
    static String run(Context c, long id, long startUs, long endUs, boolean exact, boolean keepSound, int fd) {
        Uri uri = original(c, id);
        try (ParcelFileDescriptor out = ParcelFileDescriptor.adoptFd(fd)) {
            MediaMuxer mux = new MediaMuxer(out.getFileDescriptor(), MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4);
            try {
                details(c, uri, mux);
                String err = exact ? exact(c, uri, mux, startUs, endUs, keepSound) : instant(c, uri, mux, startUs, endUs, keepSound);
                if (err == null) mux.stop();
                return err;
            } finally {
                try { mux.release(); } catch (Exception ignored) {}
            }
        } catch (Exception e) {
            Log.w("gallery", "clip failed", e);
            return FAILED;
        }
    }

    /** Which way up, and where it was filmed. */
    private static void details(Context c, Uri uri, MediaMuxer mux) {
        MediaMetadataRetriever r = new MediaMetadataRetriever();
        try {
            r.setDataSource(c, uri);
            String rot = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION);
            if (rot != null) mux.setOrientationHint(Integer.parseInt(rot));
            String loc = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_LOCATION);
            if (loc != null) {
                Matcher m = Pattern.compile("([+-][0-9.]+)([+-][0-9.]+)").matcher(loc);
                if (m.find()) {
                    float lat = Float.parseFloat(m.group(1)), lon = Float.parseFloat(m.group(2));
                    if (lat != 0f || lon != 0f) mux.setLocation(lat, lon);
                }
            }
        } catch (Exception e) {
            Log.w("gallery", "clip: reading video details failed", e);
        } finally {
            try { r.release(); } catch (Exception ignored) {}
        }
    }

    private static int track(MediaExtractor ex, String prefix) {
        for (int i = 0; i < ex.getTrackCount(); i++) {
            String mime = ex.getTrackFormat(i).getString(MediaFormat.KEY_MIME);
            if (mime != null && mime.startsWith(prefix)) return i;
        }
        return -1;
    }

    private static int bufferSize(MediaExtractor ex, int... tracks) {
        int size = 1 << 20;
        for (int t : tracks) {
            if (t < 0) continue;
            MediaFormat f = ex.getTrackFormat(t);
            if (f.containsKey(MediaFormat.KEY_MAX_INPUT_SIZE)) size = Math.max(size, f.getInteger(MediaFormat.KEY_MAX_INPUT_SIZE));
        }
        return size;
    }

    private static void progress(long done, long total) {
        if (total > 0) MainActivity.onClipProgress((int) Math.max(0, Math.min(100, done * 100 / total)));
    }

    /** Copy the stored frames, starting at the key frame at or before the start. */
    private static String instant(Context c, Uri uri, MediaMuxer mux, long startUs, long endUs, boolean keepSound) throws Exception {
        MediaExtractor ex = new MediaExtractor();
        try {
            ex.setDataSource(c, uri, null);
            int v = track(ex, "video/"), a = keepSound ? track(ex, "audio/") : -1;
            if (v < 0) return FAILED;
            // Where the copy really starts: the key frame at or before the start.
            ex.selectTrack(v);
            ex.seekTo(startUs, MediaExtractor.SEEK_TO_PREVIOUS_SYNC);
            long base = Math.max(0, ex.getSampleTime());
            int mv = mux.addTrack(ex.getTrackFormat(v));
            int ma = -1;
            if (a >= 0) {
                ex.selectTrack(a);
                ma = mux.addTrack(ex.getTrackFormat(a));
                ex.seekTo(base, MediaExtractor.SEEK_TO_PREVIOUS_SYNC);
            }
            mux.start();
            ByteBuffer buf = ByteBuffer.allocateDirect(bufferSize(ex, v, a));
            MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
            boolean vDone = false, aDone = a < 0;
            int written = 0;
            while (!(vDone && aDone)) {
                int t = ex.getSampleTrackIndex();
                if (t < 0) break;
                long time = ex.getSampleTime();
                boolean isVideo = t == v;
                if (time > endUs) {
                    if (isVideo) vDone = true; else aDone = true;
                } else if (time >= base && !(isVideo ? vDone : aDone)) {
                    info.offset = 0;
                    info.size = ex.readSampleData(buf, 0);
                    info.presentationTimeUs = time - base;
                    info.flags = (ex.getSampleFlags() & MediaExtractor.SAMPLE_FLAG_SYNC) != 0 ? MediaCodec.BUFFER_FLAG_KEY_FRAME : 0;
                    if (info.size > 0) {
                        mux.writeSampleData(isVideo ? mv : ma, buf, info);
                        written++;
                    }
                    if (isVideo) progress(time - base, endUs - base);
                }
                if (!ex.advance()) break;
            }
            return written > 0 ? null : FAILED;
        } finally {
            ex.release();
        }
    }

    /** HDR and 10-bit video: the phone's encoder can't make it again faithfully. */
    private static boolean deepColour(MediaFormat f) {
        if (f.containsKey(MediaFormat.KEY_COLOR_TRANSFER)) {
            int tr = f.getInteger(MediaFormat.KEY_COLOR_TRANSFER);
            if (tr == MediaFormat.COLOR_TRANSFER_ST2084 || tr == MediaFormat.COLOR_TRANSFER_HLG) return true;
        }
        String mime = f.getString(MediaFormat.KEY_MIME);
        if (MediaFormat.MIMETYPE_VIDEO_HEVC.equals(mime) && f.containsKey(MediaFormat.KEY_PROFILE)) {
            int p = f.getInteger(MediaFormat.KEY_PROFILE);
            if (p != MediaCodecInfo.CodecProfileLevel.HEVCProfileMain && p != MediaCodecInfo.CodecProfileLevel.HEVCProfileMainStill) return true;
        }
        return !MediaFormat.MIMETYPE_VIDEO_AVC.equals(mime) && !MediaFormat.MIMETYPE_VIDEO_HEVC.equals(mime)
                && !MediaFormat.MIMETYPE_VIDEO_MPEG4.equals(mime) && !MediaFormat.MIMETYPE_VIDEO_H263.equals(mime)
                && !MediaFormat.MIMETYPE_VIDEO_VP8.equals(mime) && !MediaFormat.MIMETYPE_VIDEO_VP9.equals(mime);
    }

    private static int number(MediaFormat f, String key, int fallback) {
        if (!f.containsKey(key)) return fallback;
        try {
            return f.getInteger(key);
        } catch (ClassCastException e) {
            return Math.round(f.getFloat(key));
        }
    }

    /** About the original's video bit rate. */
    private static int bitRate(Context c, Uri uri, MediaFormat f, int w, int h, int fps) {
        int rate = number(f, MediaFormat.KEY_BIT_RATE, 0);
        if (rate <= 0) {
            MediaMetadataRetriever r = new MediaMetadataRetriever();
            try {
                r.setDataSource(c, uri);
                String s = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_BITRATE);
                // The whole file's rate: leave some for the sound.
                if (s != null) rate = (int) (Long.parseLong(s) * 9 / 10);
            } catch (Exception ignored) {
            } finally {
                try { r.release(); } catch (Exception ignored) {}
            }
        }
        if (rate <= 0) rate = (int) Math.min(Integer.MAX_VALUE, (long) w * h * fps / 5);
        return Math.max(rate, 1_000_000);
    }

    /** Decode and encode the video so it starts and ends right at the handles. */
    private static String exact(Context c, Uri uri, MediaMuxer mux, long startUs, long endUs, boolean keepSound) throws Exception {
        MediaExtractor vx = new MediaExtractor();
        MediaExtractor ax = null;
        MediaCodec dec = null, enc = null;
        Surface surface = null;
        try {
            vx.setDataSource(c, uri, null);
            int v = track(vx, "video/");
            if (v < 0) return FAILED;
            MediaFormat in = vx.getTrackFormat(v);
            if (deepColour(in)) return CANT_EXACT;
            String mime = in.getString(MediaFormat.KEY_MIME);
            String outMime = MediaFormat.MIMETYPE_VIDEO_HEVC.equals(mime) ? MediaFormat.MIMETYPE_VIDEO_HEVC : MediaFormat.MIMETYPE_VIDEO_AVC;
            int w = in.getInteger(MediaFormat.KEY_WIDTH), h = in.getInteger(MediaFormat.KEY_HEIGHT);
            int fps = Math.max(1, number(in, MediaFormat.KEY_FRAME_RATE, 30));
            MediaFormat out = MediaFormat.createVideoFormat(outMime, w, h);
            out.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface);
            out.setInteger(MediaFormat.KEY_BIT_RATE, bitRate(c, uri, in, w, h, fps));
            out.setInteger(MediaFormat.KEY_FRAME_RATE, fps);
            out.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1);
            try {
                enc = MediaCodec.createEncoderByType(outMime);
                enc.configure(out, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE);
                surface = enc.createInputSurface();
                enc.start();
                dec = MediaCodec.createDecoderByType(mime);
                dec.configure(in, surface, null, 0);
                dec.start();
            } catch (Exception e) {
                Log.w("gallery", "clip: no codec for " + mime, e);
                return CANT_EXACT;
            }

            int ma = -1, a = -1;
            ByteBuffer abuf = null;
            if (keepSound) {
                ax = new MediaExtractor();
                ax.setDataSource(c, uri, null);
                a = track(ax, "audio/");
                if (a >= 0) {
                    ax.selectTrack(a);
                    ma = mux.addTrack(ax.getTrackFormat(a));
                    ax.seekTo(startUs, MediaExtractor.SEEK_TO_PREVIOUS_SYNC);
                    abuf = ByteBuffer.allocateDirect(bufferSize(ax, a));
                }
            }

            vx.selectTrack(v);
            vx.seekTo(startUs, MediaExtractor.SEEK_TO_PREVIOUS_SYNC);
            // Frames come out of the decoder in display order, which can run up
            // to a few frames behind the stored order: read a little past the end.
            long feedUntil = endUs + 1_000_000;
            MediaCodec.BufferInfo di = new MediaCodec.BufferInfo(), ei = new MediaCodec.BufferInfo();
            MediaCodec.BufferInfo ai = new MediaCodec.BufferInfo();
            boolean inDone = false, decDone = false, encDone = false, started = false;
            int mv = -1, frames = 0;
            long total = endUs - startUs;
            while (!encDone) {
                if (!inDone) {
                    int ii = dec.dequeueInputBuffer(WAIT_US);
                    if (ii >= 0) {
                        ByteBuffer b = dec.getInputBuffer(ii);
                        int n = b == null ? -1 : vx.readSampleData(b, 0);
                        long t = vx.getSampleTime();
                        if (n < 0 || t > feedUntil) {
                            dec.queueInputBuffer(ii, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM);
                            inDone = true;
                        } else {
                            dec.queueInputBuffer(ii, 0, n, t, 0);
                            vx.advance();
                        }
                    }
                }
                if (!decDone) {
                    int oi = dec.dequeueOutputBuffer(di, WAIT_US);
                    if (oi >= 0) {
                        long t = di.presentationTimeUs;
                        boolean keep = di.size > 0 && t >= startUs && t <= endUs;
                        // Kept frames go to the encoder with their own time;
                        // it's shifted to start at zero when written.
                        dec.releaseOutputBuffer(oi, keep);
                        if (keep) frames++;
                        if ((di.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                            enc.signalEndOfInputStream();
                            decDone = true;
                        }
                    }
                }
                int eo = enc.dequeueOutputBuffer(ei, decDone ? WAIT_US : 0);
                if (eo == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    mv = mux.addTrack(enc.getOutputFormat());
                    mux.start();
                    started = true;
                } else if (eo >= 0) {
                    ByteBuffer ob = enc.getOutputBuffer(eo);
                    if ((ei.flags & MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0) ei.size = 0;
                    if (ei.size > 0 && started && ob != null) {
                        ei.presentationTimeUs = Math.max(0, ei.presentationTimeUs - startUs);
                        mux.writeSampleData(mv, ob, ei);
                        progress(ei.presentationTimeUs, total);
                        if (ma >= 0) copySound(ax, mux, ma, abuf, ai, startUs, Math.min(endUs, startUs + ei.presentationTimeUs));
                    }
                    enc.releaseOutputBuffer(eo, false);
                    if ((ei.flags & MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) encDone = true;
                }
            }
            if (!started || frames == 0) return FAILED;
            if (ma >= 0) copySound(ax, mux, ma, abuf, ai, startUs, endUs);
            Log.i("gallery", "clip: " + frames + " frames re-encoded as " + outMime);
            return null;
        } finally {
            if (dec != null) try { dec.stop(); } catch (Exception ignored) {} finally { dec.release(); }
            if (enc != null) try { enc.stop(); } catch (Exception ignored) {} finally { enc.release(); }
            if (surface != null) surface.release();
            vx.release();
            if (ax != null) ax.release();
        }
    }

    /** Copy the sound from where `ax` is up to `untilUs`, shifted to start at zero. */
    private static void copySound(MediaExtractor ax, MediaMuxer mux, int track, ByteBuffer buf, MediaCodec.BufferInfo info, long startUs, long untilUs) {
        while (true) {
            long t = ax.getSampleTime();
            if (t < 0 || t > untilUs) return;
            if (t >= startUs) {
                info.offset = 0;
                info.size = ax.readSampleData(buf, 0);
                info.presentationTimeUs = t - startUs;
                info.flags = MediaCodec.BUFFER_FLAG_KEY_FRAME;
                if (info.size > 0) mux.writeSampleData(track, buf, info);
            }
            if (!ax.advance()) return;
        }
    }
}
