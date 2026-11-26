package dev.launcherrs.gallery;

import android.content.Context;
import android.graphics.SurfaceTexture;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.media.MediaPlayer;
import android.os.Handler;
import android.os.Looper;
import android.view.Surface;

/** One video at a time. Its frames go into a texture Rust made, so Rust
 *  draws them like anything else. Rust calls everything here from its
 *  drawing thread; Android's callbacks arrive on the main thread. */
final class Player {
    static final int PREPARED = 1, COMPLETED = 2, ERROR = 3, PAUSED = 4;

    private final Context context;
    private final AudioManager audio;
    private MediaPlayer mp;
    private SurfaceTexture st;
    private Surface surface;
    private AudioFocusRequest focus;
    private boolean prepared, muted;
    private final float[] matrix = new float[16];
    private final Handler main = new Handler(Looper.getMainLooper());

    Player(Context c) {
        context = c;
        audio = (AudioManager) c.getSystemService(Context.AUDIO_SERVICE);
    }

    synchronized void open(long id, int texName) {
        close();
        st = new SurfaceTexture(texName);
        st.setOnFrameAvailableListener(t -> MainActivity.onVideoFrame(), main);
        surface = new Surface(st);
        mp = new MediaPlayer();
        AudioAttributes attrs = new AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE).build();
        mp.setAudioAttributes(attrs);
        focus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(attrs)
                .setOnAudioFocusChangeListener(change -> {
                    if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) pause();
                }, main).build();
        final MediaPlayer p = mp;
        mp.setOnPreparedListener(x -> {
            synchronized (Player.this) {
                if (p != mp) return;
                prepared = true;
            }
            MainActivity.onVideoState(PREPARED, p.getVideoWidth(), p.getVideoHeight());
        });
        mp.setOnCompletionListener(x -> MainActivity.onVideoState(COMPLETED, 0, 0));
        mp.setOnErrorListener((x, what, extra) -> {
            MainActivity.onVideoState(ERROR, what, extra);
            return true;
        });
        // Opening the file can take a fifth of a second: not on the drawing thread.
        final Surface out = surface;
        final boolean quiet = muted;
        new Thread(() -> {
            try {
                p.setDataSource(context, Thumbs.uri(id, true));
                p.setSurface(out);
                p.setVolume(quiet ? 0f : 1f, quiet ? 0f : 1f);
                synchronized (Player.this) {
                    if (p != mp) return; // closed meanwhile
                    p.prepareAsync();
                }
            } catch (Exception e) {
                synchronized (Player.this) {
                    if (p != mp) return;
                }
                MainActivity.onVideoState(ERROR, 0, 0);
            }
        }, "video-open").start();
    }

    /** On the drawing thread: take the newest frame into the texture.
     *  Returns its texture-coordinate transform, or null with no player. */
    synchronized float[] update() {
        if (st == null) return null;
        try {
            st.updateTexImage();
        } catch (Exception e) {
            return null;
        }
        st.getTransformMatrix(matrix);
        return matrix;
    }

    synchronized void play() {
        if (mp == null || !prepared) return;
        audio.requestAudioFocus(focus);
        mp.start();
    }

    synchronized void pause() {
        if (mp == null || !prepared || !mp.isPlaying()) return;
        mp.pause();
        audio.abandonAudioFocusRequest(focus);
        MainActivity.onVideoState(PAUSED, 0, 0);
    }

    synchronized void seek(int ms) {
        if (mp != null && prepared) mp.seekTo(ms, MediaPlayer.SEEK_CLOSEST);
    }

    synchronized int position() {
        return mp != null && prepared ? mp.getCurrentPosition() : 0;
    }

    synchronized int duration() {
        return mp != null && prepared ? mp.getDuration() : 0;
    }

    synchronized boolean playing() {
        return mp != null && prepared && mp.isPlaying();
    }

    synchronized void setMuted(boolean m) {
        muted = m;
        if (mp != null) mp.setVolume(m ? 0f : 1f, m ? 0f : 1f);
    }

    synchronized void close() {
        if (mp != null) {
            mp.release();
            mp = null;
            if (focus != null) audio.abandonAudioFocusRequest(focus);
        }
        prepared = false;
        if (surface != null) surface.release();
        surface = null;
        // Releasing the SurfaceTexture detaches it from the drawing thread's
        // GL context; Rust deletes the texture itself.
        if (st != null) st.release();
        st = null;
    }
}
