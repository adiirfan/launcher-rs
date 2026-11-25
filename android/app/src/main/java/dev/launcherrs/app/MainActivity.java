package dev.launcherrs.app;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.LauncherApps;
import android.content.pm.PackageManager;
import android.content.pm.ShortcutInfo;
import android.graphics.PixelFormat;
import android.os.Bundle;
import android.os.Process;
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

    public void askContacts() {
        runOnUiThread(() -> requestPermissions(new String[] { android.Manifest.permission.READ_CONTACTS }, 1));
    }

    @Override public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
        onPermission(results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED);
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
    private static native void onPermission(boolean granted);

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
