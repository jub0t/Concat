// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.engine

import android.content.Context
import android.util.Log
import android.view.Surface
import app.concat.editor.ConcatNative
import java.io.File
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import uniffi.concat_mobile.ApiEvents
import uniffi.concat_mobile.EditorApi
import uniffi.concat_mobile.KeepAwake
import uniffi.concat_mobile.Preview
import uniffi.concat_mobile.setKeepAwake

/**
 * The engine itself, through the UniFFI bindings build-rust.sh generates.
 * Compiled only when they exist (app/build.gradle.kts); [EngineFactory]
 * finds it by name, so nothing in src/main names it.
 */
@Suppress("unused") // Made by reflection in EngineFactory.
class NativeEngineBridge(context: Context) : EngineBridge {
    override val isFake = false

    private val _events = MutableSharedFlow<String>(
        extraBufferCapacity = 256,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    override val events: SharedFlow<String> = _events

    private val api: EditorApi

    /** Held for the process: the phone's text-to-speech stays bound while it lives. */
    private val phoneVoice: AndroidPhoneVoice

    init {
        // Loads the library, and hands it the JavaVM and Context first.
        ConcatNative.init(context.applicationContext, context.filesDir.absolutePath)
        // Settings, recents, templates and models in one folder, the one the
        // Slint app used (its XDG bases were the files folder, under the
        // host's identifier), so an update over it keeps every project in
        // recents, the downloaded models and the settings.
        val home = File(context.filesDir, "app.concat.editor").apply { mkdirs() }
        api = EditorApi(
            home.absolutePath,
            home.absolutePath,
            object : ApiEvents {
                // On whichever engine thread raised it; the flow takes it from any.
                override fun event(json: String) {
                    _events.tryEmit(json)
                }
            },
        )
        // Starts the phone's text-to-speech, which offers itself to read
        // aloud once it is up.
        phoneVoice = AndroidPhoneVoice(context)
        setKeepAwake(object : KeepAwake {
            override fun stayAwake(on: Boolean) {
                Wakefulness.held.value = on
            }
        })
    }

    override fun call(request: String): String = api.call(request)

    override fun preview(projectPath: String): PreviewBridge = NativePreview(api.preview(projectPath))
}

private class NativePreview(private val preview: Preview) : PreviewBridge {
    override fun attachSurface(surface: Surface, width: Int, height: Int, mode: String) {
        val window = ConcatNative.windowFromSurface(surface)
        if (window == 0L) return
        try {
            val described = preview.attachSurface(window.toULong(), width.toUInt(), height.toUInt(), mode)
            Log.i("concat-preview", "attached ${width}x$height: $described")
        } catch (error: Exception) {
            Log.e("concat-preview", "the surface would not attach", error)
        }
    }

    override fun detachSurface() = preview.detachSurface()

    override fun refresh() {
        try {
            preview.refresh()
        } catch (error: Exception) {
            Log.e("concat-preview", "refresh failed", error)
        }
    }

    override fun play() = preview.play()
    override fun pause() = preview.pause()
    override fun seek(seconds: Double) = preview.seek(seconds)
    override fun position(): Double = preview.position()
    override fun duration(): Double = preview.duration()
    override fun isPlaying(): Boolean = preview.isPlaying()
    override fun slower(): Boolean = preview.stats().fallbacks > 0uL
    override fun release() {
        preview.detachSurface()
        preview.close()
    }
}
