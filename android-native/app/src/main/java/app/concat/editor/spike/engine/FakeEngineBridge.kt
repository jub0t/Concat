// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.engine

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.media.MediaMetadataRetriever
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import java.io.File
import kotlin.concurrent.thread
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import org.json.JSONArray
import org.json.JSONObject

/**
 * The editor without the engine: the API's JSON answered in memory, for a
 * build that has no native library. It speaks the same requests and
 * commands the real API does, for the ones the app sends, so the app above
 * it is the app that runs on the engine. Projects live as JSON in their
 * folders, recents in the config folder; the preview is a placeholder
 * painted on the Surface; an export copies the first video.
 */
class FakeEngineBridge(context: Context) : EngineBridge {
    override val isFake = true

    private val _events = MutableSharedFlow<String>(extraBufferCapacity = 256, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    override val events: SharedFlow<String> = _events

    private val config = File(context.filesDir, "config").apply { mkdirs() }
    private val recentsFile = File(config, "fake-recents.json")
    private val sessions = HashMap<String, Session>()
    private var nextId = 1
    private var nextJob = 1
    private val cancelled = HashSet<String>()

    internal class Session(val path: String, var view: JSONObject) {
        val undo = ArrayDeque<String>()
        val redo = ArrayDeque<String>()
        val project get() = view.getJSONObject("project")
        val timeline get() = project.getJSONArray("timelines").getJSONObject(0)
        val tracks get() = timeline.getJSONArray("tracks")
        val clips get() = timeline.getJSONArray("clips")
        val media get() = project.getJSONArray("media")
    }

    private class Refusal(val code: String, message: String) : Exception(message)

    @Synchronized
    override fun call(request: String): String = try {
        val json = JSONObject(request)
        JSONObject().put("result", dispatch(json)).toString()
    } catch (refusal: Refusal) {
        error(refusal.code, refusal.message ?: "")
    } catch (failure: Exception) {
        Log.w(TAG, "fake engine failed on $request", failure)
        error("invalid", failure.message ?: failure.toString())
    }

    override fun preview(projectPath: String): PreviewBridge = FakePreview(this, projectPath)

    private fun error(code: String, message: String) =
        JSONObject().put("error", JSONObject().put("code", code).put("message", message)).toString()

    private fun dispatch(request: JSONObject): Any = when (val method = request.optString("method")) {
        "version" -> JSONObject().put("apiVersion", "0.2").put("concat", "fake").put("capabilities", JSONArray().put("events"))
        "project.list" -> recents()
        "project.create" -> create(request)
        "project.open" -> open(request.getString("path")).viewOut()
        "project.get" -> session(request).viewOut()
        "project.save" -> session(request).let { write(it); JSONObject() }
        "project.close" -> {
            val session = session(request)
            if (request.optBoolean("save")) write(session)
            sessions.remove(session.path)
            JSONObject()
        }
        "edit.apply" -> session(request).let { session -> edit(session) { applyCommand(session, request.getJSONObject("command")) } }
        "edit.undo" -> session(request).let { step(it, it.undo, it.redo) }
        "edit.redo" -> session(request).let { step(it, it.redo, it.undo) }
        "media.import" -> session(request).let { session -> edit(session) { import(session, request.getString("file")) } }
        "export.run" -> export(request)
        "export.cancel" -> JSONObject().also { cancelled += request.getString("job") }
        else -> throw Refusal("invalid", "the in-memory engine does not answer $method")
    }

    private fun session(request: JSONObject): Session =
        sessions[request.getString("path")] ?: throw Refusal("notOpen", "${request.getString("path")} is not open")

    private fun Session.viewOut(createdId: String? = null): JSONObject {
        val out = JSONObject(view.toString()).put("canUndo", undo.isNotEmpty()).put("canRedo", redo.isNotEmpty())
        if (createdId != null) out.put("createdId", createdId)
        return out
    }

    // -- projects -----------------------------------------------------------

    private fun recents(): JSONArray = try {
        JSONArray(recentsFile.readText())
    } catch (_: Exception) {
        JSONArray()
    }

    private fun remember(info: JSONObject) {
        val list = recents()
        val kept = JSONArray().put(info.put("openedAt", System.currentTimeMillis()))
        for (entry in list.objectsList()) if (entry.optString("path") != info.optString("path")) kept.put(entry)
        recentsFile.writeText(kept.toString())
    }

    private fun create(request: JSONObject): JSONObject {
        val name = request.getString("name").ifBlank { "Untitled" }
        val video = request.optJSONObject("video") ?: JSONObject().put("width", 1080).put("height", 1920).put("rateNum", 30).put("rateDen", 1)
        val folder = File(request.getString("location"), name.replace(Regex("[^A-Za-z0-9 _-]"), "_"))
        if (File(folder, DOCUMENT).exists()) throw Refusal("failed", "a project is already in ${folder.path}")
        folder.mkdirs()
        val view = JSONObject()
            .put("project", JSONObject()
                .put("media", JSONArray())
                .put("activeTimelineId", "TL1")
                .put("timelines", JSONArray().put(JSONObject()
                    .put("id", "TL1").put("name", "Timeline 1").put("video", video)
                    .put("tracks", JSONArray().apply { for (i in 1..3) put(JSONObject().put("id", "T$i").put("visible", true).put("muted", false)) })
                    .put("clips", JSONArray()))))
            .put("settings", JSONObject().put("name", name).put("width", video.getInt("width")).put("height", video.getInt("height"))
                .put("rateNum", video.getLong("rateNum")).put("rateDen", video.getLong("rateDen")))
        val session = Session(folder.absolutePath, view)
        sessions[session.path] = session
        write(session)
        remember(info(session))
        return session.viewOut()
    }

    private fun open(path: String): Session {
        sessions[path]?.let { return it }
        val file = File(path, DOCUMENT)
        if (!file.exists()) throw Refusal("failed", "no project in $path")
        val session = Session(path, JSONObject(file.readText()))
        sessions[path] = session
        remember(info(session))
        return session
    }

    private fun info(session: Session): JSONObject {
        val settings = session.view.getJSONObject("settings")
        return JSONObject().put("path", session.path).put("name", settings.getString("name"))
            .put("width", settings.getInt("width")).put("height", settings.getInt("height"))
            .put("rateNum", settings.getLong("rateNum")).put("rateDen", settings.getLong("rateDen"))
            .put("canvas", JSONArray(listOf(0, 0, 0, 255)))
    }

    private fun write(session: Session) {
        File(session.path, DOCUMENT).writeText(session.view.toString())
    }

    // -- edits --------------------------------------------------------------

    /** Runs one undoable edit on a copy, committing it only if it worked. */
    private fun edit(session: Session, body: () -> String?): JSONObject {
        val before = session.view.toString()
        val created = try {
            body()
        } catch (failure: Exception) {
            session.view = JSONObject(before)
            throw failure
        }
        if (session.view.toString() != before) {
            session.undo.addLast(before)
            session.redo.clear()
        }
        return session.viewOut(created)
    }

    private fun step(session: Session, from: ArrayDeque<String>, to: ArrayDeque<String>): JSONObject {
        val state = from.removeLastOrNull() ?: return session.viewOut()
        to.addLast(session.view.toString())
        session.view = JSONObject(state)
        return session.viewOut()
    }

    private fun mint(prefix: String) = "$prefix${nextId++}"

    private fun Session.clipList() = clips.objectsList()
    private fun Session.clip(id: String) = clipList().firstOrNull { it.getString("id") == id }
    private fun Session.setClips(list: List<JSONObject>) {
        timeline.put("clips", JSONArray().apply { list.forEach { put(it) } })
    }
    private fun JSONObject.end() = getDouble("start") + getDouble("duration")

    /** Every clip on [track] starting at or after [from] moves by [delta]. */
    private fun Session.shift(track: String, from: Double, delta: Double, except: String? = null) {
        for (clip in clipList()) {
            if (clip.getString("trackId") == track && clip.getString("id") != except && clip.getDouble("start") >= from - 1e-6) {
                clip.put("start", max(0.0, clip.getDouble("start") + delta))
            }
        }
    }

    private fun Session.free(track: String, start: Double, end: Double) =
        clipList().none { it.getString("trackId") == track && it.getDouble("start") < end - 1e-6 && it.end() > start + 1e-6 }

    private fun applyCommand(session: Session, command: JSONObject): String? = with(session) {
        when (val op = command.getString("op")) {
            "batch" -> {
                var last: String? = null
                for (inner in command.getJSONArray("commands").objectsList()) last = applyCommand(session, inner) ?: last
                last
            }
            "addClip" -> {
                val media = media(command.getString("mediaId"))
                val track = command.getString("trackId")
                val start = max(0.0, command.getDouble("start"))
                val length = media.optDouble("duration", 5.0).takeIf { !it.isNaN() } ?: 5.0
                if (command.optBoolean("ripple") && !free(track, start, start + length)) shift(track, start, length)
                place(media, track, start, length)
            }
            "addClipsInSequence" -> {
                val ids = command.getJSONArray("mediaIds")
                val items = (0 until ids.length()).map { media(ids.getString(it)) }
                val lengths = items.map { it.optDouble("duration", 5.0).takeIf { d -> !d.isNaN() } ?: 5.0 }
                var at = max(0.0, command.getDouble("start"))
                val total = lengths.sum()
                val named = command.optString("trackId").takeIf { !command.isNull("trackId") && it.isNotEmpty() }
                val track = named ?: (tracks.length() - 1 downTo 0).map { tracks.getJSONObject(it).getString("id") }
                    .firstOrNull { free(it, at, at + total) } ?: addTrack(session, top = false)
                if (named != null && command.optBoolean("ripple") && !free(track, at, at + total)) shift(track, at, total)
                var last: String? = null
                items.forEachIndexed { i, item -> last = place(item, track, at, lengths[i]); at += lengths[i] }
                last
            }
            "addTextClip" -> {
                val start = max(0.0, command.getDouble("start"))
                val length = command.optDouble("duration", 3.0).takeIf { !it.isNaN() } ?: 3.0
                val style = command.optJSONObject("style") ?: JSONObject().put("content", "Your text")
                val track = command.optString("trackId").takeIf { !command.isNull("trackId") && it.isNotEmpty() }
                    ?: (0 until tracks.length()).map { tracks.getJSONObject(it).getString("id") }
                        .firstOrNull { free(it, start, start + length) } ?: addTrack(session, top = true)
                val id = mint("c")
                clips.put(JSONObject().put("id", id).put("trackId", track).put("mediaId", "")
                    .put("name", style.optString("content").lineSequence().firstOrNull().orEmpty())
                    .put("kind", "text").put("start", start).put("duration", length).put("sourceStart", 0.0)
                    .put("volume", 1.0).put("speed", 1.0).put("text", style))
                id
            }
            "splitClips" -> {
                val time = command.getDouble("time")
                val ids = command.getJSONArray("clipIds").let { a -> (0 until a.length()).map { a.getString(it) } }
                var last: String? = null
                for (id in ids) {
                    val clip = clip(id) ?: continue
                    val start = clip.getDouble("start")
                    if (time <= start + MIN || time >= clip.end() - MIN) continue
                    val tail = JSONObject(clip.toString()).put("id", mint("c")).put("start", time)
                        .put("duration", clip.end() - time)
                        .put("sourceStart", clip.getDouble("sourceStart") + (time - start) * clip.getDouble("speed"))
                    clip.put("duration", time - start)
                    clips.put(tail)
                    last = tail.getString("id")
                }
                last
            }
            "removeClips" -> {
                val ids = command.getJSONArray("clipIds").let { a -> (0 until a.length()).map { a.getString(it) }.toSet() }
                val removed = clipList().filter { it.getString("id") in ids }
                var kept = clipList().filter { it.getString("id") !in ids }
                if (command.optBoolean("ripple")) {
                    kept = kept.onEach { clip ->
                        val gone = removed.filter { it.getString("trackId") == clip.getString("trackId") && it.end() <= clip.getDouble("start") + 1e-6 }
                        clip.put("start", clip.getDouble("start") - gone.sumOf { it.getDouble("duration") })
                    }
                }
                setClips(kept)
                null
            }
            "setClipSpeed" -> {
                val clip = clip(command.getString("clipId")) ?: return null
                val speed = command.getDouble("speed").coerceIn(0.0625, 16.0)
                val old = clip.getDouble("speed")
                val end = clip.end()
                val length = max(MIN, clip.getDouble("duration") * old / speed)
                clip.put("speed", speed).put("duration", length)
                if (command.optBoolean("ripple")) shift(clip.getString("trackId"), end, clip.end() - end, except = clip.getString("id"))
                null
            }
            "trimClip" -> {
                val clip = clip(command.getString("clipId")) ?: return null
                val ripple = command.optBoolean("ripple")
                val end = clip.end()
                val delta = command.getDouble("delta")
                if (command.getString("edge") == "start") {
                    val cut = delta.coerceIn(-clip.getDouble("sourceStart") / clip.getDouble("speed"), clip.getDouble("duration") - MIN)
                    clip.put("sourceStart", clip.getDouble("sourceStart") + cut * clip.getDouble("speed"))
                        .put("duration", clip.getDouble("duration") - cut)
                    if (ripple) shift(clip.getString("trackId"), end, -cut, except = clip.getString("id"))
                    else clip.put("start", clip.getDouble("start") + cut)
                } else {
                    clip.put("duration", max(MIN, clip.getDouble("duration") + delta))
                    if (ripple) shift(clip.getString("trackId"), end, clip.end() - end, except = clip.getString("id"))
                }
                null
            }
            "updateClip" -> {
                val clip = clip(command.getString("clipId")) ?: return null
                val patch = command.getJSONObject("patch")
                if (patch.has("volume")) clip.put("volume", max(0.0, patch.getDouble("volume")))
                if (patch.has("name")) clip.put("name", patch.getString("name"))
                if (patch.has("text")) {
                    val text = patch.optJSONObject("text")
                    if (text == null) clip.remove("text") else {
                        clip.put("text", text).put("name", text.optString("content").lineSequence().firstOrNull().orEmpty())
                    }
                }
                null
            }
            else -> throw Refusal("invalid", "the in-memory engine does not apply $op")
        }
    }

    private fun Session.media(id: String): JSONObject =
        media.objectsList().firstOrNull { it.getString("id") == id } ?: throw Refusal("refused", "that media is gone")

    private fun Session.place(media: JSONObject, track: String, start: Double, length: Double): String {
        val id = mint("c")
        clips.put(JSONObject().put("id", id).put("trackId", track).put("mediaId", media.getString("id"))
            .put("name", media.getString("name")).put("kind", media.getString("kind")).put("start", start)
            .put("duration", length).put("sourceStart", 0.0).put("volume", 1.0).put("speed", 1.0))
        return id
    }

    private fun addTrack(session: Session, top: Boolean): String {
        val id = mint("t")
        val track = JSONObject().put("id", id).put("visible", true).put("muted", false)
        val list = session.tracks.objectsList().toMutableList()
        if (top) list.add(0, track) else list.add(track)
        session.timeline.put("tracks", JSONArray().apply { list.forEach { put(it) } })
        return id
    }

    private fun import(session: Session, file: String): String? {
        if (session.media.objectsList().any { it.getString("path") == file }) return null
        val source = File(file)
        if (!source.exists()) throw Refusal("failed", "$file does not exist")
        val name = source.name
        val lower = name.lowercase()
        var kind = when {
            lower.endsWith(".jpg") || lower.endsWith(".jpeg") || lower.endsWith(".png") || lower.endsWith(".webp") || lower.endsWith(".heic") -> "image"
            lower.endsWith(".mp3") || lower.endsWith(".m4a") || lower.endsWith(".wav") || lower.endsWith(".ogg") || lower.endsWith(".aac") || lower.endsWith(".flac") -> "audio"
            else -> "video"
        }
        var duration: Double? = null
        if (kind != "image") {
            try {
                MediaMetadataRetriever().apply {
                    setDataSource(file)
                    duration = extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()?.div(1000.0)
                    if (extractMetadata(MediaMetadataRetriever.METADATA_KEY_HAS_VIDEO) == null &&
                        extractMetadata(MediaMetadataRetriever.METADATA_KEY_HAS_AUDIO) != null
                    ) kind = "audio"
                    release()
                }
            } catch (failure: Exception) {
                Log.w(TAG, "could not probe $file", failure)
            }
        }
        val id = mint("m")
        session.media.put(JSONObject().put("id", id).put("path", file).put("name", name).put("kind", kind)
            .put("duration", duration ?: JSONObject.NULL).put("hasAudio", kind != "image"))
        return id
    }

    // -- export -------------------------------------------------------------

    private fun export(request: JSONObject): JSONObject {
        val session = session(request)
        val output = request.getString("output")
        val job = "j${nextJob++}"
        val source = session.media.objectsList().firstOrNull { it.getString("kind") == "video" }?.getString("path")
        val settings = session.view.getJSONObject("settings")
        val width = request.optInt("width", settings.getInt("width"))
        val height = request.optInt("height", settings.getInt("height"))
        val total = max(1, (session.clipList().maxOfOrNull { it.end() } ?: 1.0).times(30).toInt())
        val path = session.path
        thread(name = "fake-export-$job") {
            fun send(event: JSONObject) = _events.tryEmit(event.put("job", job).put("path", path).toString())
            var frame = 0
            while (frame < total) {
                if (synchronized(this) { cancelled.remove(job) }) {
                    send(JSONObject().put("event", "export.failed").put("error", JSONObject().put("code", "cancelled").put("message", "The export was cancelled.")))
                    return@thread
                }
                frame = min(total, frame + max(1, total / 60))
                send(JSONObject().put("event", "export.progress").put("frame", frame).put("total", total).put("stage", "video"))
                Thread.sleep(50)
            }
            if (source == null) {
                send(JSONObject().put("event", "export.failed").put("error", JSONObject().put("code", "failed")
                    .put("message", "The in-memory engine renders nothing; it copies the first video, and there is none.")))
                return@thread
            }
            try {
                File(output).parentFile?.mkdirs()
                File(source).copyTo(File(output), overwrite = true)
                send(JSONObject().put("event", "export.done").put("output", output).put("width", width).put("height", height))
            } catch (failure: Exception) {
                send(JSONObject().put("event", "export.failed").put("error", JSONObject().put("code", "failed").put("message", failure.message ?: "copy failed")))
            }
        }
        return JSONObject().put("job", job).put("path", path).put("output", output)
    }

    /** A snapshot of what the preview draws: the open project's view. */
    @Synchronized
    internal fun snapshot(path: String): JSONObject? = sessions[path]?.view?.let { JSONObject(it.toString()) }

    companion object {
        private const val TAG = "concat-fake"
        private const val DOCUMENT = "fake-project.json"
        private const val MIN = 1.0 / 60
    }
}

private fun JSONArray.objectsList(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }

/** A clock and a placeholder picture: the clip under the playhead, named, on its colour. */
private class FakePreview(private val bridge: FakeEngineBridge, private val path: String) : PreviewBridge {
    @Volatile private var view: JSONObject? = bridge.snapshot(path)
    @Volatile private var surface: Surface? = null
    @Volatile private var playing = false
    @Volatile private var anchor = 0.0
    @Volatile private var anchorAt = 0L
    private var painter: Thread? = null

    override fun attachSurface(surface: Surface, width: Int, height: Int, mode: String) {
        detachSurface()
        this.surface = surface
        painter = thread(name = "fake-preview") {
            while (this.surface === surface) {
                paint(surface)
                Thread.sleep(33)
            }
        }
    }

    override fun detachSurface() {
        surface = null
        painter?.join(500)
        painter = null
    }

    override fun refresh() {
        view = bridge.snapshot(path)
        if (position() > duration()) seek(duration())
    }

    override fun play() {
        if (position() >= duration() - 0.01) anchor = 0.0 else anchor = position()
        anchorAt = SystemClock.elapsedRealtimeNanos()
        playing = duration() > 0
    }

    override fun pause() {
        anchor = position()
        playing = false
    }

    override fun seek(seconds: Double) {
        anchor = seconds.coerceIn(0.0, max(0.0, duration()))
        anchorAt = SystemClock.elapsedRealtimeNanos()
    }

    override fun position(): Double {
        if (!playing) return anchor
        val at = anchor + (SystemClock.elapsedRealtimeNanos() - anchorAt) / 1e9
        if (at >= duration()) {
            playing = false
            anchor = duration()
        }
        return min(at, duration())
    }

    override fun duration(): Double {
        val clips = view?.optJSONObject("project")?.optJSONArray("timelines")?.optJSONObject(0)?.optJSONArray("clips") ?: return 0.0
        return clips.objectsList().maxOfOrNull { it.optDouble("start") + it.optDouble("duration") } ?: 0.0
    }

    override fun isPlaying() = playing && position() < duration()

    override fun release() = detachSurface()

    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE; textAlign = Paint.Align.CENTER }

    private fun paint(surface: Surface) {
        val canvas: Canvas = try {
            surface.lockCanvas(null)
        } catch (_: Exception) {
            return
        }
        try {
            canvas.drawColor(Color.BLACK)
            val timeline = view?.optJSONObject("project")?.optJSONArray("timelines")?.optJSONObject(0)
            val video = timeline?.optJSONObject("video")
            val aspect = (video?.optDouble("width", 1080.0) ?: 1080.0) / (video?.optDouble("height", 1920.0) ?: 1920.0)
            val w = canvas.width.toFloat()
            val h = canvas.height.toFloat()
            val frame = if (w / h > aspect) {
                val fw = (h * aspect).toFloat(); RectF((w - fw) / 2, 0f, (w + fw) / 2, h)
            } else {
                val fh = (w / aspect).toFloat(); RectF(0f, (h - fh) / 2, w, (h + fh) / 2)
            }
            val at = position()
            val clips = timeline?.optJSONArray("clips")?.objectsList().orEmpty()
                .filter { at >= it.optDouble("start") && at < it.optDouble("start") + it.optDouble("duration") }
            val tracks = timeline?.optJSONArray("tracks")?.objectsList().orEmpty().map { it.optString("id") }
            // Lanes are top to bottom: the picture is the lowest-indexed lane's.
            val picture = clips.filter { it.optString("kind") == "video" || it.optString("kind") == "image" }
                .minByOrNull { tracks.indexOf(it.optString("trackId")) }
            fill.color = if (picture == null) Color.rgb(24, 24, 24) else
                Color.HSVToColor(floatArrayOf((picture.optString("mediaId").hashCode() and 0xff) * 360f / 256f, 0.45f, 0.45f))
            canvas.drawRect(frame, fill)
            text.textSize = frame.height() * 0.03f
            text.color = Color.argb(160, 255, 255, 255)
            canvas.drawText(picture?.optString("name") ?: "no picture here", frame.centerX(), frame.bottom - text.textSize * 2, text)
            canvas.drawText("in-memory engine  %.2f s".format(at), frame.centerX(), frame.top + text.textSize * 2, text)
            for (title in clips.filter { it.optString("kind") == "text" }) {
                val style = title.optJSONObject("text") ?: continue
                text.textSize = (style.optDouble("fontSize", 0.09) * frame.height()).toFloat()
                text.color = try {
                    Color.parseColor(style.optString("color", "#ffffff"))
                } catch (_: Exception) {
                    Color.WHITE
                }
                var y = frame.centerY()
                for (line in style.optString("content").lines()) {
                    canvas.drawText(line, frame.centerX(), y, text)
                    y += text.textSize * 1.2f
                }
            }
        } finally {
            try {
                surface.unlockCanvasAndPost(canvas)
            } catch (_: Exception) {
            }
        }
    }
}
