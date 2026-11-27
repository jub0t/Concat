// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui.editor

import android.app.Application
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.SystemClock
import android.provider.OpenableColumns
import android.util.Log
import android.view.Surface
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.concat.editor.api.AppliedEffect
import app.concat.editor.api.ClipBounds
import app.concat.editor.api.ClipKind
import app.concat.editor.api.Commands
import app.concat.editor.api.JobTracker
import app.concat.editor.api.ProjectView
import app.concat.editor.api.ShapeCard
import app.concat.editor.api.errorMessage
import app.concat.editor.api.wasCancelled
import app.concat.editor.concat
import app.concat.editor.engine.PreviewBridge
import app.concat.editor.export.EXPORT_FPS_OPTIONS
import app.concat.editor.export.ExportFps
import app.concat.editor.export.ExportPreset
import app.concat.editor.subtitle.SubtitleCodec
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
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
    /** The preview draws the slower way: the phone's graphics failed under it. */
    val previewSlower: Boolean = false,
    val busy: Boolean = false,
    val message: String? = null,
    /** The clip a reverse is being written for, and how far it is (0..1). */
    val reversing: Pair<String, Float>? = null,
    /** Each title's and shape's block as the engine painted it, for the transform box. */
    val bounds: Map<String, ClipBounds> = emptyMap(),
    /** The Stickers page's cards, once the engine has said them. */
    val stickers: List<ShapeCard> = emptyList(),
    /** The cover the project names, painted; none while it names none (the timeline shows its first frame). */
    val cover: Bitmap? = null,
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
    val models get() = app.models

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
        viewModelScope.launch {
            ui.map { it.view }.distinctUntilChanged().collectLatest { view ->
                if (view == null) return@collectLatest
                try {
                    val bounds = api.bounds(path)
                    _ui.update { it.copy(bounds = bounds) }
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Exception) {
                    Log.w(TAG, "clips.bounds failed", error)
                }
            }
        }
        viewModelScope.launch {
            ui.map { it.view?.cover?.toString() }.distinctUntilChanged().collectLatest { named ->
                if (named == null) {
                    _ui.update { it.copy(cover = null) }
                    return@collectLatest
                }
                try {
                    val file = File(getApplication<Application>().cacheDir, "cover-${path.hashCode()}.png")
                    api.projectCover(path, file.absolutePath, COVER_SIDE, COVER_SIDE)
                    val painted = withContext(Dispatchers.IO) { BitmapFactory.decodeFile(file.absolutePath) }
                    _ui.update { it.copy(cover = painted) }
                } catch (error: CancellationException) {
                    throw error
                } catch (error: Exception) {
                    Log.w(TAG, "project.cover failed", error)
                }
            }
        }
        viewModelScope.launch {
            try {
                val cards = api.shapes()
                _ui.update { it.copy(stickers = cards) }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                Log.w(TAG, "shapes.list failed", error)
            }
        }
        viewModelScope.launch(Dispatchers.IO) {
            for (seconds in seeks) preview?.seek(seconds)
        }
        viewModelScope.launch {
            while (true) {
                val shown = preview
                if (shown != null && SystemClock.uptimeMillis() - scrubbedAt > 300) {
                    val (at, playing, slower) = withContext(Dispatchers.IO) {
                        Triple(shown.position(), shown.isPlaying(), shown.slower())
                    }
                    _ui.update { it.copy(position = at, playing = playing, previewSlower = slower) }
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

    fun edit(label: String, body: suspend () -> ProjectView?) = change(label, body)

    fun move(clipId: String, start: Double, target: LaneTarget) {
        val at = start.coerceAtLeast(0.0)
        change("Move") {
            when (target) {
                is LaneTarget.Existing -> apply(Commands.moveClips(listOf(Triple(clipId, at, target.trackId))))
                LaneTarget.NewTop, LaneTarget.NewBottom ->
                    apply(Commands.moveClipsToNewTrack(listOf(clipId to at), bottom = target == LaneTarget.NewBottom))
            }
        }
    }

    fun trim(clipId: String, edge: String, delta: Double) {
        change("Trim") { apply(Commands.trimClip(clipId, edge, delta, ripple = false)) }
    }

    fun placeSound(file: String, at: Double, label: String) {
        change(label) {
            val after = api.import(path, file)
            val id = after.createdId ?: after.media.firstOrNull { it.path == file || File(it.path).name == File(file).name }?.id
                ?: return@change after
            val view = apply(Commands.addClipsInSequence(listOf(id), at))
            view.createdId?.let { created -> _ui.update { it.copy(selected = created) } }
            view
        }
    }

    // -- effects, filters, transitions --

    fun applyPackage(kind: String, packageId: String, name: String) {
        if (kind == "transition") {
            val clip = selectedClip()
            if (clip?.kind == ClipKind.Transition) {
                val patch = JSONObject().put("name", name)
                    .put("videoEffects", org.json.JSONArray().put(AppliedEffect.fresh(packageId)))
                change("Transition") { apply(Commands.updateClip(clip.id, patch)) }
                return
            }
            val at = clip?.start ?: _ui.value.position
            val start = (at - 0.5).coerceAtLeast(0.0)
            change("Transition") {
                val view = apply(Commands.addTransitionClip(packageId, name, start, duration = 1.0))
                view.createdId?.let { id -> _ui.update { it.copy(selected = id) } }
                view
            }
            return
        }
        val clip = selectedClip() ?: return say("Select a clip first.")
        val field = if (kind == "audio") "filters" else "videoEffects"
        val chain = (if (kind == "audio") clip.filters else clip.videoEffects).map { it.raw }
        change(name) { apply(Commands.setChain(clip.id, field, chain + AppliedEffect.fresh(packageId))) }
    }

    private val liveChains = Channel<Triple<String, String, List<JSONObject>>>(Channel.CONFLATED)
    private val liveText = Channel<Pair<String, JSONObject>>(Channel.CONFLATED)

    init {
        viewModelScope.launch {
            for ((clipId, field, chain) in liveChains) {
                edits.withLock {
                    try {
                        val view = apply(Commands.setChain(clipId, field, chain))
                        withContext(Dispatchers.IO) { preview?.refresh() }
                        _ui.update { it.copy(view = view) }
                    } catch (error: CancellationException) {
                        throw error
                    } catch (error: Exception) {
                        Log.w(TAG, "live parameter failed", error)
                    }
                }
            }
        }
        viewModelScope.launch {
            for ((clipId, style) in liveText) {
                edits.withLock {
                    try {
                        val existing = _ui.value.view?.clip(clipId) ?: continue@withLock
                        val lane = _ui.value.view?.tracks?.firstOrNull { it.id == existing.trackId }
                        val captions = lane?.captions
                        val view = if (lane != null && captions != null) {
                            val words = Commands.updateClip(existing.id, JSONObject().put("text", style))
                            val shared = JSONObject(style.toString()).put("content", "")
                            val look = JSONObject(captions.toString()).put("style", shared)
                            apply(Commands.batch(listOf(words, Commands.setCaptionLook(lane.id, look))))
                        } else {
                            apply(Commands.updateClip(existing.id, JSONObject().put("text", style)))
                        }
                        withContext(Dispatchers.IO) { preview?.refresh() }
                        _ui.update { it.copy(view = view) }
                    } catch (error: CancellationException) {
                        throw error
                    } catch (error: Exception) {
                        Log.w(TAG, "live text failed", error)
                    }
                }
            }
        }
    }

    /** Memperbarui gaya & teks klip secara real-time saat mengetik atau menggeser slider/eyedropper. */
    fun previewText(clipId: String, style: JSONObject) {
        liveText.trySend(clipId to style)
    }

    fun setLink(clipId: String, field: String, index: Int, link: JSONObject) {
        val clip = _ui.value.view?.clip(clipId) ?: return
        val chain = (if (field == "filters") clip.filters else clip.videoEffects).map { it.raw }.toMutableList()
        if (index !in chain.indices) return
        chain[index] = link
        liveChains.trySend(Triple(clipId, field, chain))
    }

    fun removeLink(clipId: String, field: String, index: Int) {
        val clip = _ui.value.view?.clip(clipId) ?: return
        val chain = (if (field == "filters") clip.filters else clip.videoEffects).map { it.raw }.toMutableList()
        if (index !in chain.indices) return
        chain.removeAt(index)
        change("Remove") { apply(Commands.setChain(clipId, field, chain)) }
    }

    fun removeLinks(clipId: String, links: List<Pair<String, Int>>) {
        val clip = _ui.value.view?.clip(clipId) ?: return
        if (links.isEmpty()) return
        val commands = listOf("videoEffects", "filters").mapNotNull { field ->
            val gone = links.filter { it.first == field }.map { it.second }.toSet()
            if (gone.isEmpty()) return@mapNotNull null
            val chain = (if (field == "filters") clip.filters else clip.videoEffects).map { it.raw }
            Commands.setChain(clipId, field, chain.filterIndexed { i, _ -> i !in gone })
        }
        change("Remove") { apply(if (commands.size == 1) commands[0] else Commands.batch(commands)) }
    }

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

    fun duplicate() {
        val clip = selectedClip() ?: return
        change("Duplicate") {
            val view = apply(Commands.duplicateClips(listOf(clip.id)))
            view.createdId?.let { id -> _ui.update { it.copy(selected = id) } }
            view
        }
    }

    fun freeze() {
        val clip = selectedClip() ?: return
        val at = _ui.value.position
        val edge = 1.0 / (_ui.value.view?.fps ?: 30.0)
        if (at <= clip.start + edge || at >= clip.end - edge) return say("Move the playhead inside the clip to freeze it.")
        change("Freeze") {
            val view = api.clipsFreeze(path, clip.id, at)
            view.clips.firstOrNull { it.trackId == clip.trackId && kotlin.math.abs(it.start - at) < 1e-6 }
                ?.let { hold -> _ui.update { it.copy(selected = hold.id) } }
            view
        }
    }

    fun replace(uri: Uri?) {
        val clip = selectedClip() ?: return
        if (uri == null) return
        change("Replace") {
            val file = withContext(Dispatchers.IO) { copyIn(uri) }
            api.clipsReplace(path, clip.id, file.absolutePath)
        }
    }

    private val reverses = JobTracker("reverse", ::onReverse)

    init {
        viewModelScope.launch { api.rawEvents.collect { reverses.offer(it) } }
    }

    fun reverse() {
        val clip = selectedClip() ?: return
        if (_ui.value.reversing != null) return say("A reverse is already running.")
        _ui.update { it.copy(reversing = clip.id to 0f) }
        reverses.expect()
        viewModelScope.launch {
            try {
                reverses.start(api.clipsReverse(path, clip.id))
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                reverses.stop()
                _ui.update { it.copy(reversing = null) }
                say(error.message ?: "The reverse did not start.")
            }
        }
    }

    fun cancelReverse() {
        val job = reverses.job ?: return
        viewModelScope.launch {
            try {
                api.reverseCancel(job)
            } catch (error: Exception) {
                Log.w(TAG, "reverse cancel failed", error)
            }
        }
    }

    private fun onReverse(event: JSONObject) {
        when (event.optString("event")) {
            "reverse.progress" -> _ui.update { ui ->
                ui.copy(reversing = ui.reversing?.copy(second = event.optDouble("fraction", 0.0).toFloat()))
            }
            "reverse.done" -> {
                reverses.stop()
                _ui.update { it.copy(reversing = null) }
                change("Reverse") { api.clipsUseReversed(path, event) }
            }
            "reverse.failed" -> {
                reverses.stop()
                _ui.update { it.copy(reversing = null) }
                if (!event.wasCancelled()) say(event.errorMessage())
            }
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

    fun setOpacity(opacity: Double) {
        val clip = selectedClip() ?: return
        change("Opacity") { apply(Commands.updateClip(clip.id, JSONObject().put("opacity", opacity.coerceIn(0.0, 1.0)))) }
    }

    fun setFades(fadeIn: Double, fadeOut: Double) {
        val clip = selectedClip() ?: return
        change("Fade") { apply(Commands.updateClip(clip.id, JSONObject().put("fadeIn", fadeIn).put("fadeOut", fadeOut))) }
    }

    fun setLook(label: String, patch: JSONObject) {
        val clip = selectedClip() ?: return
        change(label) { apply(Commands.updateClip(clip.id, patch)) }
    }

    fun toggleMute() {
        val clip = selectedClip() ?: return
        change(if (clip.muted) "Unmute" else "Mute") { apply(Commands.updateClip(clip.id, JSONObject().put("muted", !clip.muted))) }
    }

    fun setTrackSoundMuted(trackId: String, muted: Boolean) {
        val clips = _ui.value.view?.clips?.filter { it.trackId == trackId && it.kind == ClipKind.Video }.orEmpty()
        if (clips.isEmpty()) return
        change(if (muted) "Mute" else "Unmute") {
            apply(Commands.batch(clips.map { Commands.updateClip(it.id, JSONObject().put("muted", muted)) }))
        }
    }

    fun flip() {
        val clip = selectedClip() ?: return
        change("Flip") { apply(Commands.updateClip(clip.id, JSONObject().put("flipH", !clip.flipH))) }
    }

    fun setTransform(clipId: String, scale: Double, offsetX: Double, offsetY: Double, rotation: Double) {
        change("Move") { apply(Commands.setClipTransform(clipId, scale, offsetX, offsetY, rotation)) }
    }

    fun resetTransform() {
        val clip = selectedClip() ?: return
        setTransform(clip.id, 1.0, 0.0, 0.0, 0.0)
    }

    fun toggleKeyframe() {
        val clip = selectedClip() ?: return
        if (clip.duration <= 0) return
        val at = ((_ui.value.position - clip.start) / clip.duration).coerceIn(0.0, 1.0)
        val tolerance = 0.5 / ((_ui.value.view?.fps ?: 30.0) * clip.duration)
        val properties = listOf(
            "scale" to clip.scale, "offsetX" to clip.offsetX, "offsetY" to clip.offsetY,
            "rotation" to clip.rotation, "opacity" to clip.opacity,
        )
        if (clip.keyedAt(at, tolerance)) {
            val commands = clip.keys.flatMap { (property, ats) ->
                ats.filter { kotlin.math.abs(it - at) <= tolerance }.map { Commands.clearClipKey(clip.id, property, it) }
            }
            change("Keyframe") { apply(Commands.batch(commands)) }
        } else {
            change("Keyframe") { apply(Commands.batch(properties.map { (name, value) -> Commands.setClipKey(clip.id, name, at, value) })) }
        }
    }

    fun setAspect(width: Int, height: Int) {
        val view = _ui.value.view ?: return
        if (view.width == width && view.height == height) return
        change("Aspect") { api.setVideo(path, width, height, view.rateNum, view.rateDen) }
    }

    fun setBackground(background: JSONObject) {
        val view = _ui.value.view ?: return
        change("Background") { apply(Commands.setTimelineBackground(view.timelineId, background)) }
    }

    fun rename(name: String) {
        val clean = name.trim()
        if (clean.isEmpty()) return
        change("Rename") {
            api.rename(path, clean)
            api.get(path)
        }
    }

    fun selectAtPlayhead(trackId: String?) {
        val view = _ui.value.view ?: return
        val at = _ui.value.position
        val clip = view.clips.firstOrNull { (trackId == null || it.trackId == trackId) && at >= it.start && at < it.end }
            ?: view.clips.filter { trackId == null || it.trackId == trackId }.minByOrNull { kotlin.math.abs(it.start - at) }
        select(clip?.id)
    }

    /** A new title at the playhead, or the selected one restyled when [clipId] names it. */
    fun saveText(clipId: String?, style: JSONObject) {
        val content = style.optString("content").trim()
        if (content.isEmpty()) return
        val cleanStyle = JSONObject(style.toString()).put("content", content)
        val existing = _ui.value.view?.clip(clipId)
        val lane = existing?.let { clip -> _ui.value.view?.tracks?.firstOrNull { it.id == clip.trackId } }
        val captions = lane?.captions
        change("Text") {
            if (existing != null && lane != null && captions != null) {
                val words = Commands.updateClip(existing.id, JSONObject().put("text", cleanStyle))
                val shared = JSONObject(cleanStyle.toString()).put("content", "")
                val look = JSONObject(captions.toString()).put("style", shared)
                apply(Commands.batch(listOf(words, Commands.setCaptionLook(lane.id, look))))
            } else if (existing != null) {
                apply(Commands.updateClip(existing.id, JSONObject().put("text", cleanStyle)))
            } else {
                val view = apply(Commands.addTextClip(_ui.value.position, cleanStyle))
                view.createdId?.let { id -> _ui.update { it.copy(selected = id) } }
                view
            }
        }
    }

    // -- eyedropper & subtitles --

    /** Mengambil frame komposit pada waktu aktif sebagai Bitmap untuk Eyedropper. */
    suspend fun grabEyedropperFrame(): Bitmap? = withContext(Dispatchers.IO) {
        val view = _ui.value.view ?: return@withContext null
        val tall = view.height >= view.width
        val w = if (tall) 360 else 640
        val h = (w * view.height.toDouble() / view.width.coerceAtLeast(1)).toInt().coerceAtLeast(2)
        val out = File(getApplication<Application>().cacheDir, "eyedropper-${path.hashCode()}.png")
        try {
            api.previewFrameTo(path, _ui.value.position, out.absolutePath, w, h)
            BitmapFactory.decodeFile(out.absolutePath)
        } catch (error: Exception) {
            Log.w(TAG, "eyedropper frame failed", error)
            null
        }
    }

    /** Mengimpor file .srt atau .ass menjadi multi-track text clip tanpa mem-flatten waktu yang overlap. */
    fun importSubtitles(uri: Uri?) {
        if (uri == null) return
        val view = _ui.value.view ?: return
        change("Import Subtitles") {
            val (rawText, fileName) = withContext(Dispatchers.IO) {
                val resolver = getApplication<Application>().contentResolver
                var name = "subtitle.srt"
                resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                    if (it.moveToFirst()) name = it.getString(0) ?: name
                }
                val text = resolver.openInputStream(uri)?.bufferedReader(Charsets.UTF_8)?.use { it.readText() }
                    ?: throw IOException("Tidak dapat membaca file subtitle.")
                text to name
            }
            val cues = SubtitleCodec.parse(rawText, fileName, view.height)
            if (cues.isEmpty()) {
                say("Tidak ada baris subtitle yang terbaca.")
                return@change null
            }
            val cmds = SubtitleCodec.buildImportCommands(cues)
            val updated = apply(Commands.batch(cmds))
            say("Berhasil impor ${cues.size} baris subtitle.")
            updated
        }
    }

    /** Mengekspor semua track teks/subtitle ke file .srt atau .ass yang dipilih user. */
    fun exportSubtitles(uri: Uri?, asAss: Boolean) {
        if (uri == null) return
        val view = _ui.value.view ?: return
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val content = if (asAss) {
                    SubtitleCodec.exportAss(view)
                } else {
                    SubtitleCodec.exportSrt(view)
                }
                getApplication<Application>().contentResolver.openOutputStream(uri)?.bufferedWriter(Charsets.UTF_8)?.use {
                    it.write(content)
                } ?: throw IOException("Gagal membuka tujuan simpan.")
                say(if (asAss) "File .ASS berhasil diekspor." else "File .SRT berhasil diekspor.")
            } catch (error: Exception) {
                Log.w(TAG, "export subtitles failed", error)
                say(error.message ?: "Gagal mengekspor subtitle.")
            }
        }
    }

    // -- cover --

    fun setCoverFrame() {
        val at = _ui.value.position
        change("Cover") { apply(Commands.setCover(time = at)) }
    }

    fun setCoverPicture(uri: Uri?) {
        if (uri == null) return
        change("Cover") {
            val file = withContext(Dispatchers.IO) { copyIn(uri) }
            apply(Commands.setCover(picture = file.absolutePath))
        }
    }

    fun resetCover() {
        if (_ui.value.view?.cover == null) return
        change("Cover") { apply(Commands.setCover()) }
    }

    fun addSticker(card: ShapeCard, name: String) {
        change("Sticker") {
            val view = apply(Commands.addShapeClip(_ui.value.position, card.style, name))
            view.createdId?.let { id -> _ui.update { it.copy(selected = id) } }
            view
        }
    }

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

    fun importOverlay(uris: List<Uri>) {
        if (uris.isEmpty()) return
        change("Overlay") {
            var view: ProjectView? = null
            for (uri in uris) {
                val file = withContext(Dispatchers.IO) { copyIn(uri) }
                val after = api.import(path, file.absolutePath)
                val id = after.createdId ?: after.media.firstOrNull { it.path == file.absolutePath || File(it.path).name == file.name }?.id
                    ?: continue
                val lane = apply(Commands.addTrack(bottom = false))
                val trackId = lane.createdId ?: lane.tracks.last().id
                view = apply(Commands.addClip(id, trackId, _ui.value.position, ripple = false))
                view.createdId?.let { created -> _ui.update { it.copy(selected = created) } }
            }
            view
        }
    }

    private fun copyIn(uri: Uri): File {
        val resolver = getApplication<Application>().contentResolver
        var name = "picked-${System.currentTimeMillis()}"
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) name = it.getString(0) ?: name
        }
        val target = File(app.mediaDir, "${System.currentTimeMillis()}-${name.replace('/', '_')}")
        val input = resolver.openInputStream(uri) ?: throw IOException("The provider would not open $uri")
        input.use { stream -> target.outputStream().use { stream.copyTo(it, 1 shl 20) } }
        return target
    }

    // -- export --

    fun export(preset: ExportPreset, fps: ExportFps = EXPORT_FPS_OPTIONS[0]) {
        val view = _ui.value.view ?: return
        pause()
        viewModelScope.launch {
            try {
                api.save(path)
            } catch (error: Exception) {
                Log.w(TAG, "save before export failed", error)
            }
            exports.start(path, view.name.ifBlank { "Concat" }, view.width, view.height, preset, fps)
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
                    writeCard()
                    api.close(path, save = true)
                } catch (error: Exception) {
                    Log.w(TAG, "close failed", error)
                }
            }
            done()
        }
    }

    private suspend fun writeCard() {
        val view = _ui.value.view ?: return
        val folder = File(path)
        try {
            File(folder, CARD).writeText(
                JSONObject().put("duration", view.duration).put("clips", view.clips.size)
                    .put("savedAt", System.currentTimeMillis()).toString(),
            )
        } catch (error: Exception) {
            Log.w(TAG, "no card for $path", error)
        }
        if (view.clips.isEmpty()) {
            File(folder, THUMBNAIL).delete()
            return
        }
        val tall = view.height >= view.width
        val width = if (tall) 270 else 480
        val height = (width * view.height.toDouble() / view.width.coerceAtLeast(1)).toInt()
        try {
            api.projectCover(path, File(folder, THUMBNAIL).absolutePath, width, height)
        } catch (error: Exception) {
            Log.w(TAG, "no thumbnail for $path: ${error.message}")
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
        private const val MODE = "vulkan"
        const val THUMBNAIL = ".concat-thumb.png"
        const val CARD = ".concat-card.json"
        private const val COVER_SIDE = 160
    }
}
