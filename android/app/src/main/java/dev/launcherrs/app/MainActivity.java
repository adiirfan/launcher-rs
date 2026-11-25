package dev.launcherrs.app;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.PixelFormat;
import android.os.Bundle;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.*;

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
            else imm.hideSoftInputFromWindow(input.getWindowToken(), 0);
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
