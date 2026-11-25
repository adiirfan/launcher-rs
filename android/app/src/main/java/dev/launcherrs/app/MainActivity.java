package dev.launcherrs.app;

import android.app.NativeActivity;
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
 *  only hands keyboard text to Rust and shows or hides the keyboard when Rust asks. */
public class MainActivity extends NativeActivity {
    static { System.loadLibrary("launcher"); } // so Java finds onText/onAction
    private View input;

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().setFormat(PixelFormat.TRANSLUCENT); // let the wallpaper show through
        input = new View(this) {
            @Override public boolean onCheckIsTextEditor() { return true; }
            @Override public InputConnection onCreateInputConnection(EditorInfo info) {
                info.inputType = InputType.TYPE_CLASS_TEXT;
                info.imeOptions = EditorInfo.IME_ACTION_SEARCH | EditorInfo.IME_FLAG_NO_FULLSCREEN;
                return new Bridge(this);
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

    private static native void onText(String text, int selStart, int selEnd, int compStart, int compEnd);
    private static native void onAction();

    private static final class Bridge extends BaseInputConnection {
        Bridge(View v) { super(v, true); }
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
