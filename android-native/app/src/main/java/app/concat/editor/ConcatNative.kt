package app.concat.editor

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

    /**
     * The log - a file in [files], which Settings shares, and logcat - and
     * the JavaVM + Context that cpal's AAudio host reads.
     */
    @JvmStatic external fun init(context: Context, files: String)

    /** The Surface's ANativeWindow with one reference acquired; 0 for none. */
    @JvmStatic external fun windowFromSurface(surface: Surface): Long
}
