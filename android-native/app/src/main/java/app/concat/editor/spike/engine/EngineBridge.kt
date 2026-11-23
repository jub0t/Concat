// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.engine

import android.content.Context
import android.util.Log
import android.view.Surface
import kotlinx.coroutines.flow.SharedFlow

/**
 * The editor as the app holds it: the API's one JSON call, its events, and
 * a preview per open project. Two implementations: [FakeEngineBridge],
 * in memory, for a build without the engine, and `NativeEngineBridge`
 * (src/native/java) over the UniFFI `EditorApi`.
 *
 * Every method but [events] may block; call them off the main thread,
 * except [PreviewBridge.attachSurface] and [PreviewBridge.detachSurface],
 * which belong to the SurfaceHolder callbacks that must wait for them.
 */
interface EngineBridge {
    /** True for the in-memory stand-in. */
    val isFake: Boolean

    /** Every event the API raises, as its JSON object. */
    val events: SharedFlow<String>

    /** One request, `{"method": ...}`, and its reply: `{"result": ...}` or `{"error": ...}`. */
    fun call(request: String): String

    /** The picture and sound of an open project. */
    fun preview(projectPath: String): PreviewBridge
}

/** One project's picture and sound. */
interface PreviewBridge {
    /** Draws on [surface]; `mode` is "vulkan", "gl" or "cpu". */
    fun attachSurface(surface: Surface, width: Int, height: Int, mode: String)

    /** Returns once nothing draws on the surface any more. */
    fun detachSurface()

    /** Reads the edit again: after every change sent through the API. */
    fun refresh()
    fun play()
    fun pause()
    fun seek(seconds: Double)
    fun position(): Double
    fun duration(): Double
    fun isPlaying(): Boolean

    /** Lets the preview's device and audio stream go. */
    fun release()
}

/** Picks the native bridge when it was compiled in and its library loads. */
object EngineFactory {
    private const val TAG = "concat-engine"

    fun create(context: Context): EngineBridge {
        try {
            val native = Class.forName("app.concat.editor.spike.engine.NativeEngineBridge")
            return native.getConstructor(Context::class.java).newInstance(context) as EngineBridge
        } catch (_: ClassNotFoundException) {
            Log.i(TAG, "built without the engine's bindings: the in-memory engine stands in")
        } catch (error: Throwable) {
            // A library that will not load (wrong ABI, a missing .so) is the
            // fake engine, not a dead app.
            Log.e(TAG, "the native engine would not start; the in-memory engine stands in", error)
        }
        return FakeEngineBridge(context)
    }
}
