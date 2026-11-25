package dev.launcherrs.app;

import android.app.NativeActivity;
import android.content.ComponentName;
import android.content.ContentUris;
import android.content.Intent;
import android.content.pm.LauncherApps;
import android.content.pm.PackageManager;
import android.content.pm.ShortcutInfo;
import android.database.Cursor;
import android.graphics.PixelFormat;
import android.media.MediaMetadata;
import android.media.session.MediaController;
import android.media.session.MediaSessionManager;
import android.media.session.PlaybackState;
import android.net.Uri;
import android.os.Bundle;
import android.os.Process;
import android.provider.CalendarContract;
import android.provider.Settings;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.view.HapticFeedbackConstants;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.*;
import java.util.ArrayList;
import java.util.List;

/** Android needs a Java class to start. All logic lives in Rust (liblauncher); this
 *  only passes events to Rust (keyboard text, Home pressed, permission answers)
 *  and does the few things Android only allows on its UI thread. */
public class MainActivity extends NativeActivity {
    static { System.loadLibrary("launcher"); } // so Java finds onText/onAction
    private View input;
    /** The search text. Kept here so it survives the keyboard reconnecting
     *  (after a dialog or the notification shade takes focus). */
    private final Editable text = Editable.Factory.getInstance().newEditable("");

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().setFormat(PixelFormat.TRANSLUCENT); // let the wallpaper show through
        input = new View(this) {
            @Override public boolean onCheckIsTextEditor() { return true; }
            @Override public InputConnection onCreateInputConnection(EditorInfo info) {
                info.inputType = InputType.TYPE_CLASS_TEXT;
                info.imeOptions = EditorInfo.IME_ACTION_SEARCH | EditorInfo.IME_FLAG_NO_FULLSCREEN;
                return new Bridge(this, text);
            }
        };
        input.setFocusableInTouchMode(true);
        addContentView(input, new ViewGroup.LayoutParams(1, 1));
    }

    /** Called from Rust's thread; the keyboard may only be touched on the UI thread. */
    public void showKeyboard(boolean show) {
        runOnUiThread(() -> {
            InputMethodManager imm = (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
            if (show) { input.requestFocus(); imm.showSoftInput(input, 0); }
            else {
                imm.hideSoftInputFromWindow(input.getWindowToken(), 0);
                // Let go of the text box too, or Android brings the keyboard
                // back whenever Home comes to the front again.
                input.clearFocus();
            }
        });
    }

    /** Empty the search text, and tell the keyboard to start over. */
    public void clearText() {
        runOnUiThread(() -> {
            BaseInputConnection.removeComposingSpans(text);
            text.clear();
            Selection.setSelection(text, 0);
            ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE)).restartInput(input);
            onText("", 0, 0, -1, -1);
        });
    }

    /** Home pressed while we're already Home. */
    @Override protected void onNewIntent(Intent intent) { super.onNewIntent(intent); onHome(); }

    /** Ask for one permission; the answer goes to Rust with the same code. */
    public void askPermission(String permission, int code) {
        runOnUiThread(() -> requestPermissions(new String[] { permission }, code));
    }

    @Override public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
        onPermission(code, results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED);
    }

    /** The next calendar event in the coming day that hasn't ended, as
     *  "start-millis TAB title · time", or "" when there is none. */
    public String nextEvent() {
        if (checkSelfPermission(android.Manifest.permission.READ_CALENDAR) != PackageManager.PERMISSION_GRANTED) return "";
        long now = System.currentTimeMillis(), day = 24L * 60 * 60 * 1000;
        Uri.Builder range = CalendarContract.Instances.CONTENT_URI.buildUpon();
        ContentUris.appendId(range, now - day);
        ContentUris.appendId(range, now + day);
        String[] cols = { CalendarContract.Instances.TITLE, CalendarContract.Instances.BEGIN };
        String where = CalendarContract.Instances.END + " > ? AND " + CalendarContract.Instances.ALL_DAY + " = 0 AND "
                + CalendarContract.Instances.VISIBLE + " = 1";
        try (Cursor c = getContentResolver().query(range.build(), cols, where, new String[] { Long.toString(now) },
                CalendarContract.Instances.BEGIN + " ASC")) {
            if (c == null || !c.moveToFirst()) return "";
            String title = c.getString(0);
            long begin = c.getLong(1);
            if (title == null || title.trim().isEmpty()) title = "Event";
            String when;
            if (begin <= now) when = "now";
            else {
                String time = android.text.format.DateFormat.getTimeFormat(this).format(new java.util.Date(begin));
                when = android.text.format.DateUtils.isToday(begin) ? time : "tomorrow " + time;
            }
            return begin + "\t" + title.trim().replace('\t', ' ').replace('\n', ' ') + " \u00B7 " + when;
        } catch (RuntimeException e) {
            return "";
        }
    }

    /** Whether the owner has allowed notification access (dots and music). */
    public boolean hasListenerAccess() {
        String on = Settings.Secure.getString(getContentResolver(), "enabled_notification_listeners");
        return on != null && on.contains(new ComponentName(this, DotsListener.class).flattenToString());
    }

    public void openListenerSettings() {
        runOnUiThread(() -> {
            try { startActivity(new Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")); }
            catch (RuntimeException e) { /* no such page on this phone */ }
        });
    }

    // Music: follow the playing (or else the newest) media session and tell
    // Rust its title, artist and whether it plays.
    private MediaController media;
    private boolean watchingMedia;
    private final MediaController.Callback mediaChanged = new MediaController.Callback() {
        @Override public void onPlaybackStateChanged(PlaybackState s) { pushMedia(); }
        @Override public void onMetadataChanged(MediaMetadata m) { pushMedia(); }
        @Override public void onSessionDestroyed() { setMedia(null); }
    };

    public void watchMedia() {
        runOnUiThread(() -> {
            if (watchingMedia) return;
            try {
                MediaSessionManager m = (MediaSessionManager) getSystemService(MEDIA_SESSION_SERVICE);
                ComponentName me = new ComponentName(this, DotsListener.class);
                m.addOnActiveSessionsChangedListener(this::pickMedia, me);
                pickMedia(m.getActiveSessions(me));
                watchingMedia = true;
            } catch (RuntimeException e) { /* notification access not allowed yet */ }
        });
    }

    private void pickMedia(java.util.List<MediaController> sessions) {
        MediaController best = null;
        if (sessions != null) for (MediaController c : sessions) {
            PlaybackState s = c.getPlaybackState();
            if (s != null && s.getState() == PlaybackState.STATE_PLAYING) { best = c; break; }
            if (best == null) best = c;
        }
        setMedia(best);
    }

    private void setMedia(MediaController c) {
        if (media != null) media.unregisterCallback(mediaChanged);
        media = c;
        if (media != null) media.registerCallback(mediaChanged);
        pushMedia();
    }

    private void pushMedia() {
        String title = "", artist = "";
        int state = 0;
        if (media != null) {
            MediaMetadata m = media.getMetadata();
            if (m != null) {
                CharSequence t = m.getText(MediaMetadata.METADATA_KEY_TITLE), a = m.getText(MediaMetadata.METADATA_KEY_ARTIST);
                title = t == null ? "" : t.toString();
                artist = a == null ? "" : a.toString();
            }
            PlaybackState s = media.getPlaybackState();
            if (s != null) state = s.getState() == PlaybackState.STATE_PLAYING ? 1 : s.getState() == PlaybackState.STATE_PAUSED ? 2 : 0;
        }
        onMedia(title, artist, state);
    }

    /** 0 previous, 1 play or pause, 2 next, 3 open the music app. */
    public void mediaButton(int button) {
        runOnUiThread(() -> {
            if (media == null) return;
            MediaController.TransportControls t = media.getTransportControls();
            PlaybackState s = media.getPlaybackState();
            boolean playing = s != null && s.getState() == PlaybackState.STATE_PLAYING;
            try {
                if (button == 0) t.skipToPrevious();
                else if (button == 2) t.skipToNext();
                else if (button == 1) { if (playing) t.pause(); else t.play(); }
                else if (media.getSessionActivity() != null) media.getSessionActivity().send();
                else {
                    Intent i = getPackageManager().getLaunchIntentForPackage(media.getPackageName());
                    if (i != null) startActivity(i);
                }
            } catch (Exception e) { /* the player went away */ }
        });
    }

    /** The app's own shortcuts (from its manifest first, then the ones it
     *  added while running), at most four, as "id\tlabel" lines. Empty when
     *  Android won't tell us (only the default Home may ask). */
    public String shortcuts(String pkg) {
        try {
            LauncherApps la = (LauncherApps) getSystemService(LAUNCHER_APPS_SERVICE);
            if (!la.hasShortcutHostPermission()) return "";
            LauncherApps.ShortcutQuery q = new LauncherApps.ShortcutQuery();
            q.setPackage(pkg);
            q.setQueryFlags(LauncherApps.ShortcutQuery.FLAG_MATCH_MANIFEST | LauncherApps.ShortcutQuery.FLAG_MATCH_DYNAMIC);
            List<ShortcutInfo> all = la.getShortcuts(q, Process.myUserHandle());
            if (all == null) return "";
            List<ShortcutInfo> sorted = new ArrayList<>(all);
            sorted.sort((a, b) -> a.isDeclaredInManifest() != b.isDeclaredInManifest()
                    ? (a.isDeclaredInManifest() ? -1 : 1) : Integer.compare(a.getRank(), b.getRank()));
            StringBuilder out = new StringBuilder();
            int n = 0;
            for (ShortcutInfo s : sorted) {
                if (!s.isEnabled() || n == 4) continue;
                CharSequence label = s.getShortLabel();
                if (label == null || label.length() == 0) label = s.getLongLabel();
                if (label == null || label.length() == 0) continue;
                out.append(s.getId().replaceAll("[\t\n]", " ")).append('\t')
                   .append(label.toString().replaceAll("[\t\n]", " ")).append('\n');
                n++;
            }
            return out.toString();
        } catch (RuntimeException e) {
            return "";
        }
    }

    public boolean startShortcut(String pkg, String id) {
        try {
            ((LauncherApps) getSystemService(LAUNCHER_APPS_SERVICE)).startShortcut(pkg, id, null, null, Process.myUserHandle());
            return true;
        } catch (RuntimeException e) {
            return false;
        }
    }

    /** The short buzz of a long-press. Uses the phone's touch-feedback
     *  setting, so it needs no permission. */
    public void buzz() {
        runOnUiThread(() -> getWindow().getDecorView().performHapticFeedback(HapticFeedbackConstants.LONG_PRESS));
    }

    /** Android's own "choose a wallpaper" list. */
    public void pickWallpaper() {
        runOnUiThread(() -> {
            try {
                startActivity(Intent.createChooser(new Intent(Intent.ACTION_SET_WALLPAPER), null));
            } catch (RuntimeException e) { /* nothing on the phone offers wallpapers */ }
        });
    }

    private static native void onText(String text, int selStart, int selEnd, int compStart, int compEnd);
    private static native void onAction();
    private static native void onHome();
    private static native void onPermission(int code, boolean granted);
    private static native void onMedia(String title, String artist, int state);

    private static final class Bridge extends BaseInputConnection {
        private final Editable text;
        Bridge(View v, Editable text) { super(v, true); this.text = text; }
        @Override public Editable getEditable() { return text; }
        private boolean push(boolean r) {
            Editable e = getEditable();
            onText(e.toString(), Selection.getSelectionStart(e), Selection.getSelectionEnd(e),
                    getComposingSpanStart(e), getComposingSpanEnd(e));
            return r;
        }
        @Override public boolean commitText(CharSequence t, int p) { return push(super.commitText(t, p)); }
        @Override public boolean setComposingText(CharSequence t, int p) { return push(super.setComposingText(t, p)); }
        @Override public boolean finishComposingText() { return push(super.finishComposingText()); }
        @Override public boolean deleteSurroundingText(int b, int a) { return push(super.deleteSurroundingText(b, a)); }
        @Override public boolean setSelection(int s, int e) { return push(super.setSelection(s, e)); }
        @Override public boolean performEditorAction(int action) { onAction(); return true; }
        /** Some keyboards send keys instead of text: Backspace, Enter, plain letters. */
        @Override public boolean sendKeyEvent(KeyEvent k) {
            if (k.getAction() != KeyEvent.ACTION_DOWN) return true;
            Editable e = getEditable();
            int s = Selection.getSelectionStart(e), end = Selection.getSelectionEnd(e);
            switch (k.getKeyCode()) {
                case KeyEvent.KEYCODE_DEL:
                    if (s != end) { e.delete(Math.min(s, end), Math.max(s, end)); return push(true); }
                    return push(super.deleteSurroundingTextInCodePoints(1, 0));
                case KeyEvent.KEYCODE_ENTER: onAction(); return true;
                default:
                    int u = k.getUnicodeChar();
                    return u == 0 || commitText(new String(Character.toChars(u)), 1);
            }
        }
    }
}
