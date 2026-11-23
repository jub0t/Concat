// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import android.app.Application
import android.net.Uri
import android.os.SystemClock
import android.provider.OpenableColumns
import android.util.Log
import android.view.Surface
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.concat.editor.spike.api.ClipKind
import app.concat.editor.spike.api.Commands
import app.concat.editor.spike.api.ProjectView
import app.concat.editor.spike.concat
import app.concat.editor.spike.engine.PreviewBridge
import app.concat.editor.spike.export.ExportPreset
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject

data class EditorUi(
    val view: ProjectView? = null,
    val selected: String? = null,
    val position: Double = 0.0,
    val playing: Boolean = false,
    val busy: Boolean = false,
    val message: String? = null,
)

/**
 * One open project: its state as the API last replied, the playhead, the
 * selection and the preview. Every edit is `edit.apply` (or undo/redo),
 * then `preview.refresh()`, and the reply is the new state, so the screen
 * never guesses. Engine calls run on the IO dispatcher, one edit at a time.
 */
class EditorViewModel(application: Application, val path: String) : AndroidViewModel(application) {
    private val app = application.concat
    private val api = app.api
    private val _ui = MutableStateFlow(EditorUi())
    val ui: StateFlow<EditorUi> = _ui
    val exports get() = app.exports

    @Volatile private var preview: PreviewBridge? = null
    /** The surface waiting for the preview, when it came first. */
    private var pending: Triple<Surface, Int, Int>? = null
    private val edits = Mutex()
    private val seeks = Channel<Double>(Channel.CONFLATED)
    @Volatile private var scrubbedAt = 0L
    private var closed = false

    init {
        viewModelScope.launch {
            try {
                val view = api.open(path)
                _ui.update { it.copy(view = view) }
                val made = withContext(Dispatchers.IO) { api.engine.preview(path).also { it.refresh() } }
                preview = made
                pending?.let { (surface, width, height) -> made.attachSurface(surface, width, height, MODE) }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                Log.e(TAG, "could not open $path", error)
                say(error.message ?: "The project would not open.")
            }
        }
        // Scrubs, the latest one only: a drag sends far more than a frame can show.
        viewModelScope.launch(Dispatchers.IO) {
            for (seconds in seeks) preview?.seek(seconds)
        }
        // The playhead, from the engine's clock while it plays.
        viewModelScope.launch {
            while (true) {
                val shown = preview
                if (shown != null && SystemClock.uptimeMillis() - scrubbedAt > 300) {
                    val (at, playing) = withContext(Dispatchers.IO) { shown.position() to shown.isPlaying() }
                    _ui.update { it.copy(position = at, playing = playing) }
                }
                delay(if (_ui.value.playing) 33 else 250)
            }
        }
    }

    // -- the preview's surface, from the SurfaceHolder callbacks (main thread) --

    fun surfaceChanged(surface: Surface, width: Int, height: Int) {
        pending = Triple(surface, width, height)
        preview?.attachSurface(surface, width, height, MODE)
    }

    fun surfaceDestroyed() {
        pending = null
        // Must return only once nothing draws on it.
        preview?.detachSurface()
    }

    // -- transport --

    fun togglePlay() {
        val shown = preview ?: return
        viewModelScope.launch(Dispatchers.IO) {
            if (shown.isPlaying()) shown.pause() else shown.play()
            _ui.update { it.copy(playing = shown.isPlaying()) }
        }
    }

    fun pause() {
        val shown = preview ?: return
        viewModelScope.launch(Dispatchers.IO) {
            shown.pause()
            _ui.update { it.copy(playing = false) }
        }
    }

    /** Moves the playhead to [seconds] at once, the frame as soon as the engine can. */
    fun scrubTo(seconds: Double) {
        val duration = _ui.value.view?.duration ?: 0.0
        val at = seconds.coerceIn(0.0, duration)
        scrubbedAt = SystemClock.uptimeMillis()
        if (_ui.value.playing) pause()
        _ui.update { it.copy(position = at) }
        seeks.trySend(at)
    }

    fun select(clipId: String?) = _ui.update { it.copy(selected = clipId) }

    // -- edits --

    private fun change(label: String, body: suspend () -> ProjectView?) {
        viewModelScope.launch {
            edits.withLock {
                _ui.update { it.copy(busy = true) }
                try {
                    val view = body()
                    if (view != null) {
                        withContext(Dispatchers.IO) { preview?.refresh() }
                        _ui.update { ui ->
                            ui.copy(
                                view = view,
                                selected = ui.selected?.takeIf { view.clip(it) != null },
                                position = ui.position.coerceAtMost(view.duration),
                            )
                        }
                    }
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Exception) {
                    Log.w(TAG, "$label failed", error)
                    say(error.message ?: "$label did not work.")
                } finally {
                    _ui.update { it.copy(busy = false) }
                }
            }
        }
    }

    private suspend fun apply(command: JSONObject) = api.apply(path, command)

    fun undo() = change("Undo") { api.undo(path) }
    fun redo() = change("Redo") { api.redo(path) }

    fun split() {
        val clip = selectedClip() ?: return
        change("Split") { apply(Commands.splitClips(listOf(clip.id), _ui.value.position)) }
    }

    fun delete() {
        val clip = selectedClip() ?: return
        _ui.update { it.copy(selected = null) }
        change("Delete") { apply(Commands.removeClips(listOf(clip.id), ripple = true)) }
    }

    /**
     * A copy of the whole clip right after it, on its lane, in one undo
     * step: the edit model's own `duplicateClips`.
     */
    fun duplicate() {
        val clip = selectedClip() ?: return
        change("Duplicate") {
            val view = apply(Commands.duplicateClips(listOf(clip.id)))
            view.createdId?.let { id -> _ui.update { it.copy(selected = id) } }
            view
        }
    }

    fun setSpeed(speed: Double) {
        val clip = selectedClip() ?: return
        change("Speed") { apply(Commands.setClipSpeed(clip.id, speed)) }
    }

    fun setVolume(volume: Double) {
        val clip = selectedClip() ?: return
        change("Volume") { apply(Commands.updateClip(clip.id, JSONObject().put("volume", volume))) }
    }

    /** A new title at the playhead, or the selected one restyled when [clipId] names it. */
    fun saveText(clipId: String?, content: String, fontSize: Double, color: String) {
        if (content.isBlank()) return
        val existing = _ui.value.view?.clip(clipId)
        change("Text") {
            if (existing != null) {
                apply(Commands.updateClip(existing.id, JSONObject().put("text", Commands.textStyle(content, fontSize, color, existing.text))))
            } else {
                val view = apply(Commands.addTextClip(_ui.value.position, Commands.textStyle(content, fontSize, color)))
                view.createdId?.let { id -> _ui.update { it.copy(selected = id) } }
                view
            }
        }
    }

    /**
     * Picked files copied into the app's storage (the engine reads paths),
     * imported one by one in the order picked, then laid down as one run:
     * pictures after the last picture, sound at the playhead.
     */
    fun import(uris: List<Uri>, audio: Boolean) {
        if (uris.isEmpty()) return
        change("Import") {
            var view: ProjectView? = null
            val ids = mutableListOf<String>()
            for (uri in uris) {
                val file = withContext(Dispatchers.IO) { copyIn(uri) }
                val after = api.import(path, file.absolutePath)
                view = after
                val id = after.createdId ?: after.media.firstOrNull { it.path == file.absolutePath || File(it.path).name == file.name }?.id
                if (id != null) ids += id
            }
            if (ids.isEmpty()) return@change view
            val start = if (audio) _ui.value.position else view?.pictureEnd ?: 0.0
            apply(Commands.addClipsInSequence(ids, start))
        }
    }

    private fun copyIn(uri: Uri): File {
        val resolver = getApplication<Application>().contentResolver
        var name = "picked-${System.currentTimeMillis()}"
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) name = it.getString(0) ?: name
        }
        // Each pick its own file: two picks of one name are two files.
        val target = File(app.mediaDir, "${System.currentTimeMillis()}-${name.replace('/', '_')}")
        val input = resolver.openInputStream(uri) ?: throw IOException("The provider would not open $uri")
        input.use { stream -> target.outputStream().use { stream.copyTo(it, 1 shl 20) } }
        return target
    }

    // -- export --

    fun export(preset: ExportPreset) {
        val view = _ui.value.view ?: return
        pause()
        viewModelScope.launch {
            // The document on disk first: what renders is what was saved.
            try {
                api.save(path)
            } catch (error: Exception) {
                Log.w(TAG, "save before export failed", error)
            }
            exports.start(path, view.name.ifBlank { "Concat" }, view.width, view.height, preset)
                .onFailure { say(it.message ?: "The export did not start.") }
        }
    }

    // -- lifecycle --

    fun save() {
        if (closed) return
        app.scope.launch {
            try {
                api.save(path)
            } catch (error: Exception) {
                Log.w(TAG, "save failed", error)
            }
        }
    }

    /** Back: save, close the project, let the preview go, then [done]. */
    fun close(done: () -> Unit) {
        if (closed) return done()
        closed = true
        val shown = preview
        preview = null
        viewModelScope.launch {
            withContext(NonCancellable + Dispatchers.IO) {
                try {
                    shown?.pause()
                    shown?.release()
                    api.close(path, save = true)
                } catch (error: Exception) {
                    Log.w(TAG, "close failed", error)
                }
            }
            done()
        }
    }

    override fun onCleared() {
        val shown = preview
        preview = null
        if (shown != null) app.scope.launch(Dispatchers.IO) { shown.release() }
    }

    fun dismissMessage() = _ui.update { it.copy(message = null) }
    private fun say(message: String) = _ui.update { it.copy(message = message) }
    private fun selectedClip() = _ui.value.view?.clip(_ui.value.selected)

    companion object {
        private const val TAG = "concat-editor"
        /** Vulkan, falling back to OpenGL ES inside the engine. */
        private const val MODE = "vulkan"
    }
}
