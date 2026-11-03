package app.concat.editor.spike

import android.content.Context
import android.view.Surface

/**
 * The two JNI entry points UniFFI cannot carry (src/crates/concat-mobile/src/jni.rs).
 * Loading the library here, through System.loadLibrary, is also what runs it
 * once before JNA opens the same .so for the UniFFI calls.
 */
object ConcatNative {
    init {
        System.loadLibrary("concat_mobile")
    }

    /** Logcat, and the JavaVM + Context that cpal's AAudio host reads. */
    @JvmStatic external fun init(context: Context)

    /** The Surface's ANativeWindow with one reference acquired; 0 for none. */
    @JvmStatic external fun windowFromSurface(surface: Surface): Long
}
