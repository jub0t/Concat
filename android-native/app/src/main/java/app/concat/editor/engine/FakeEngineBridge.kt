// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.engine

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

    /** The Stickers page's ids, in the engine's order (ShapeStyle::CARDS). */
    private val stickerIds = listOf(
        "square", "rounded", "circle", "ring", "triangle", "pentagon", "polygon", "star", "heart",
        "pill", "parallelogram", "trapezoid", "line", "arrow", "double-arrow",
    )

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
        "project.save" -> session(request).let { session ->
            request.optString("name").takeIf { !request.isNull("name") && it.isNotBlank() }?.let { name ->
                session.view.getJSONObject("settings").put("name", name.trim())
                remember(info(session))
            }
            write(session)
            JSONObject()
        }
        "project.setVideo" -> session(request).let { session -> edit(session) { setVideo(session, request.getJSONObject("video")) } }
        "media.filmstrip" -> FakePictures.filmstrip(request)
        "media.peaks" -> FakePictures.peaks(request, mediaDuration(request.getString("file")))
        "preview.frame" -> previewFrame(request)
        "project.cover" -> projectCover(request)
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
        "catalogue.list" -> FakeCatalogue.list(request.optString("kind").ifEmpty { null })
        "models.list" -> modelsList()
        "shapes.list" -> JSONArray().apply {
            for (id in stickerIds) put(JSONObject().put("id", id).put("style", JSONObject().put("kind", id).put("fill", "#ffffff").put("width", 0.35)))
        }
        "clips.bounds" -> session(request).let { session -> fakeBounds(session) }
        "models.download" -> modelsDownload(request.getString("kind"), request.getString("id"))
        "models.delete" -> JSONObject().also { setDownloaded(request.getString("id"), false) }
        "captions.transcribe" -> transcribe(request)
        "captions.place" -> session(request).let { session -> edit(session) { placeCaptions(session, request) } }
        "captions.cancel", "speech.cancel" -> JSONObject().also { cancelled += request.getString("job") }
        "speech.speak" -> speak(request)
        "clips.freeze" -> session(request).let { session -> edit(session) { freeze(session, request) } }
        "clips.replace" -> session(request).let { session -> edit(session) { replace(session, request) } }
        "clips.reverse" -> reverse(request)
        "clips.useReversed" -> session(request).let { session ->
            edit(session) {
                // The fake writes no copy: the clip is only marked as shown backwards.
                val clip = session.clip(request.getString("clipId")) ?: throw Refusal("invalid", "no such clip")
                clip.put("name", clip.optString("name").removeSuffix(" (reversed)") + " (reversed)")
                null
            }
        }
        "reverse.cancel" -> JSONObject().also { cancelled += request.getString("job") }
        else -> throw Refusal("invalid", "the in-memory engine does not answer $method")
    }

    // -- frames, strips and outlines ----------------------------------------

    private fun setVideo(session: Session, video: JSONObject): String? {
        val width = video.optInt("width")
        val height = video.optInt("height")
        if (width <= 0 || height <= 0) throw Refusal("invalid", "a frame needs a size")
        val current = session.timeline.optJSONObject("video") ?: JSONObject()
        current.put("width", width).put("height", height)
            .put("rateNum", video.optLong("rateNum", current.optLong("rateNum", 30)))
            .put("rateDen", video.optLong("rateDen", current.optLong("rateDen", 1)))
        session.timeline.put("video", current)
        session.view.getJSONObject("settings").put("width", width).put("height", height)
        return null
    }

    /** A file's length, from any open project's bin, or by asking the file. */
    private fun mediaDuration(file: String): Double {
        for (session in sessions.values) {
            session.media.objectsList().firstOrNull { it.optString("path") == file }?.let { item ->
                if (!item.isNull("duration")) return item.optDouble("duration", 5.0)
            }
        }
        return try {
            MediaMetadataRetriever().run {
                setDataSource(file)
                val ms = extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
                release()
                (ms ?: 5000L) / 1000.0
            }
        } catch (_: Exception) {
            5.0
        }
    }

    /** The placeholder picture at an instant: what the preview paints, as a PNG. */
    /** `project.cover`: the picture set as the cover, cut to fill; else the frame it names, or one early in. */
    private fun projectCover(request: JSONObject): JSONObject {
        val session = session(request)
        val cover = session.project.optJSONObject("cover")
        if (cover?.optString("kind") == "picture") {
            val source = android.graphics.BitmapFactory.decodeFile(cover.optString("path"))
                ?: throw Refusal("failed", "${cover.optString("path")} would not open")
            val width = request.optInt("width", 270).coerceIn(8, 2048)
            val height = request.optInt("height", 480).coerceIn(8, 2048)
            val scale = maxOf(width.toFloat() / source.width, height.toFloat() / source.height)
            val bitmap = android.graphics.Bitmap.createBitmap(width, height, android.graphics.Bitmap.Config.ARGB_8888)
            val shownW = source.width * scale
            val shownH = source.height * scale
            val target = android.graphics.RectF((width - shownW) / 2, (height - shownH) / 2, (width + shownW) / 2, (height + shownH) / 2)
            Canvas(bitmap).drawBitmap(source, null, target, android.graphics.Paint(android.graphics.Paint.FILTER_BITMAP_FLAG))
            val output = request.getString("output")
            File(output).apply { parentFile?.mkdirs() }.outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            return JSONObject().put("path", output).put("width", width).put("height", height)
        }
        val length = session.clips.objectsList().maxOfOrNull { it.getDouble("start") + it.getDouble("duration") } ?: 0.0
        val time = if (cover?.optString("kind") == "frame") cover.optDouble("time", 0.0).coerceAtMost(length) else (length * 0.25).coerceAtMost(2.0)
        return previewFrame(JSONObject(request.toString()).put("time", time))
    }

    private fun previewFrame(request: JSONObject): JSONObject {
        val path = request.getString("path")
        val view = sessions[path]?.view ?: File(path, DOCUMENT).takeIf { it.exists() }?.let { JSONObject(it.readText()) }
            ?: throw Refusal("notOpen", "$path is not a project")
        val video = view.optJSONObject("project")?.optJSONArray("timelines")?.optJSONObject(0)?.optJSONObject("video")
        val frameWidth = video?.optInt("width", 1080) ?: 1080
        val frameHeight = video?.optInt("height", 1920) ?: 1920
        var width = request.optInt("width", 0)
        var height = request.optInt("height", 0)
        if (width <= 0 && height <= 0) { width = frameWidth / 4; height = frameHeight / 4 }
        if (width <= 0) width = (height * frameWidth.toDouble() / frameHeight).toInt()
        if (height <= 0) height = (width * frameHeight.toDouble() / frameWidth).toInt()
        val bitmap = android.graphics.Bitmap.createBitmap(width.coerceIn(8, 2048), height.coerceIn(8, 2048), android.graphics.Bitmap.Config.ARGB_8888)
        FakeFrame.paint(Canvas(bitmap), view, request.optDouble("time", 0.0), showClock = false)
        val bytes = java.io.ByteArrayOutputStream().also { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        val output = request.optString("output").takeIf { !request.isNull("output") && it.isNotEmpty() }
        return if (output != null) {
            File(output).apply { parentFile?.mkdirs() }.writeBytes(bytes)
            JSONObject().put("output", output).put("width", bitmap.width).put("height", bitmap.height)
        } else {
            JSONObject().put("width", bitmap.width).put("height", bitmap.height)
                .put("png", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP))
        }
    }

    // -- speech models, captions, reading aloud ----------------------------

    private val modelsFile = File(config, "fake-models.json")

    private fun downloadedModels(): MutableSet<String> = try {
        JSONArray(modelsFile.readText()).let { a -> (0 until a.length()).map { a.getString(it) }.toMutableSet() }
    } catch (_: Exception) {
        mutableSetOf()
    }

    private fun setDownloaded(id: String, downloaded: Boolean) {
        val set = downloadedModels()
        if (downloaded) set += id else set -= id
        modelsFile.writeText(JSONArray(set.toList()).toString())
    }

    /** `clips.bounds` from a reading of each style: about as the painter would, enough for the emulator's box. */
    private fun fakeBounds(session: Session): JSONArray {
        val video = session.timeline.optJSONObject("video") ?: JSONObject()
        val aspect = video.optDouble("width", 1080.0) / video.optDouble("height", 1920.0).coerceAtLeast(1.0)
        return JSONArray().apply {
            for (clip in session.clips.objectsList()) {
                val (width, height) = when (clip.optString("kind")) {
                    "text" -> {
                        val style = clip.optJSONObject("text") ?: JSONObject()
                        val em = style.optDouble("fontSize", 0.09)
                        val lines = style.optString("content").lines().ifEmpty { listOf("") }
                        val longest = lines.maxOf { it.length }.coerceAtLeast(1)
                        (longest * em * 0.6 + em * 0.6) / aspect to lines.size * em * 1.2 + em * 0.5
                    }
                    "shape" -> {
                        val side = clip.optJSONObject("shape")?.optDouble("width", 0.35) ?: 0.35
                        side / aspect to side
                    }
                    else -> continue
                }
                put(JSONObject().put("clipId", clip.getString("id")).put("width", width).put("height", height)
                    .put("offsetX", 0.0).put("offsetY", 0.0))
            }
        }
    }

    private fun modelsList(): JSONObject {
        val have = downloadedModels()
        fun model(id: String, label: String, blurb: String, family: String, englishOnly: Boolean, size: Long) =
            JSONObject().put("id", id).put("label", label).put("blurb", blurb).put("family", family)
                .put("englishOnly", englishOnly).put("sizeBytes", size).put("downloaded", id in have)
        return JSONObject()
            .put("transcribers", JSONArray()
                .put(model("tiny.en", "Tiny (English)", "Fastest; misses more words.", "", true, 75_000_000))
                .put(model("base", "Base", "Any language; a good balance.", "", false, 142_000_000))
                .put(model("small", "Small", "Hears best; slowest on a phone.", "", false, 466_000_000)))
            .put("speech", JSONArray()
                .put(model("system", "Phone voice", "The voice the phone has; nothing to download.", "system", false, 0)
                    .put("downloaded", true))
                .put(model("chatterbox", "Chatterbox", "Natural voices; a large download.", "chatterbox", false, 520_000_000)))
            .put("voices", JSONArray()
                .put(JSONObject().put("id", 0).put("name", "Default").put("family", "system"))
                .put(JSONObject().put("id", 1).put("name", "Ava").put("family", "chatterbox"))
                .put(JSONObject().put("id", 2).put("name", "Noah").put("family", "chatterbox"))
                .put(JSONObject().put("id", 3).put("name", "Mia").put("family", "chatterbox")))
    }

    private fun modelsDownload(kind: String, id: String): JSONObject {
        if (kind != "transcriber" && kind != "speech") throw Refusal("invalid", "$kind is not a model kind")
        val size = modelsList().let { all ->
            (all.getJSONArray("transcribers").objectsList() + all.getJSONArray("speech").objectsList())
                .firstOrNull { it.getString("id") == id }?.getLong("sizeBytes")
        } ?: throw Refusal("notFound", "no model $id")
        val job = "j${nextJob++}"
        thread(name = "fake-model-$job") {
            fun send(event: JSONObject) = _events.tryEmit(event.put("job", job).put("id", id).toString())
            val total = max(1L, size)
            for (step in 1..40) {
                if (synchronized(this) { cancelled.remove(job) }) {
                    send(JSONObject().put("event", "models.failed").put("error", JSONObject().put("code", "cancelled").put("message", "Cancelled.")))
                    return@thread
                }
                send(JSONObject().put("event", "models.progress").put("received", total * step / 40).put("total", total).put("unpacking", step == 40))
                Thread.sleep(60)
            }
            synchronized(this) { setDownloaded(id, true) }
            send(JSONObject().put("event", "models.done"))
        }
        return JSONObject().put("job", job).put("path", "").put("output", "")
    }

    private fun transcribe(request: JSONObject): JSONObject {
        val session = session(request)
        val clipId = request.getString("clipId")
        val clip = session.clip(clipId) ?: throw Refusal("notFound", "no clip $clipId")
        val model = request.getString("model")
        if (model !in downloadedModels()) throw Refusal("refused", "$model is not downloaded")
        val job = "j${nextJob++}"
        val path = session.path
        val heard = clip.getDouble("duration") * clip.getDouble("speed")
        val from = clip.getDouble("sourceStart")
        thread(name = "fake-captions-$job") {
            fun send(event: JSONObject) = _events.tryEmit(event.put("job", job).put("path", path).toString())
            val stages = listOf("reading" to 6, "loading" to 6, "hearing" to 30)
            for ((stage, steps) in stages) {
                for (step in 1..steps) {
                    if (synchronized(this) { cancelled.remove(job) }) {
                        send(JSONObject().put("event", "captions.failed").put("error", JSONObject().put("code", "cancelled").put("message", "Cancelled.")))
                        return@thread
                    }
                    send(JSONObject().put("event", "captions.progress").put("stage", stage)
                        .put("fraction", if (stage == "hearing") step.toDouble() / steps else 0.0))
                    Thread.sleep(70)
                }
            }
            val segments = JSONArray()
            var at = 0.0
            var line = 0
            while (at < heard - 0.2) {
                val end = min(heard, at + 2.0)
                segments.put(JSONObject().put("start", from + at).put("end", from + end).put("text", FAKE_WORDS[line % FAKE_WORDS.size]))
                at = end
                line++
            }
            send(JSONObject().put("event", "captions.done").put("clipId", clipId).put("segments", segments))
        }
        return JSONObject().put("job", job).put("path", path).put("output", "")
    }

    private fun placeCaptions(session: Session, request: JSONObject): String? {
        val clip = session.clip(request.getString("clipId")) ?: throw Refusal("notFound", "that clip is gone")
        val size = when (request.optString("size", "medium")) { "small" -> 0.045; "large" -> 0.09; else -> 0.065 }
        val offsetY = when (request.optString("placement", "bottom")) { "top" -> -0.35; "middle" -> 0.0; else -> 0.35 }
        val speed = clip.getDouble("speed")
        val sourceStart = clip.getDouble("sourceStart")
        var last: String? = null
        val segments = request.getJSONArray("segments").objectsList()
        if (segments.isEmpty()) return null
        val first = clip.getDouble("start") + (segments.minOf { it.getDouble("start") } - sourceStart) / speed
        val lastEnd = clip.getDouble("start") + (segments.maxOf { it.getDouble("end") } - sourceStart) / speed
        val track = session.firstFreeAbove(first, lastEnd)
        for (segment in segments) {
            val text = segment.optString("text").trim()
            if (text.isEmpty()) continue
            val start = clip.getDouble("start") + (segment.getDouble("start") - sourceStart) / speed
            val length = max(MIN, (segment.getDouble("end") - segment.getDouble("start")) / speed)
            val style = JSONObject().put("content", text).put("fontSize", size).put("color", "#ffffff")
            val id = mint("c")
            session.clips.put(JSONObject().put("id", id).put("trackId", track).put("mediaId", "").put("name", text)
                .put("kind", "text").put("start", max(0.0, start)).put("duration", length).put("sourceStart", 0.0)
                .put("volume", 1.0).put("speed", 1.0).put("text", style).put("offsetY", offsetY))
            last = id
        }
        // A caption lane: its look shared, as the engine's PlaceCaptions makes it.
        session.tracks.objectsList().firstOrNull { it.optString("id") == track }?.put(
            "captions",
            JSONObject().put("style", JSONObject().put("fontSize", size).put("color", "#ffffff")).put("offsetY", offsetY),
        )
        return last
    }

    private fun speak(request: JSONObject): JSONObject {
        val session = session(request)
        val text = request.getString("text").trim()
        if (text.isEmpty()) throw Refusal("invalid", "there is nothing to say")
        val model = request.getString("model")
        if (model != "system" && model !in downloadedModels()) throw Refusal("refused", "$model is not downloaded")
        val speed = request.optDouble("speed", 1.0).takeIf { !it.isNaN() && it > 0 } ?: 1.0
        val job = "j${nextJob++}"
        val path = session.path
        val seconds = max(1.0, text.split(Regex("\\s+")).size * 0.38 / speed)
        val file = File(path, "speech-${System.currentTimeMillis()}.wav")
        thread(name = "fake-speech-$job") {
            fun send(event: JSONObject) = _events.tryEmit(event.put("job", job).put("path", path).toString())
            for (step in 1..25) {
                if (synchronized(this) { cancelled.remove(job) }) {
                    send(JSONObject().put("event", "speech.failed").put("error", JSONObject().put("code", "cancelled").put("message", "Cancelled.")))
                    return@thread
                }
                send(JSONObject().put("event", "speech.progress").put("stage", if (step < 5) "loading" else "reading").put("fraction", step / 25.0))
                Thread.sleep(60)
            }
            try {
                writeTone(file, seconds)
                send(JSONObject().put("event", "speech.done").put("file", file.absolutePath).put("duration", seconds))
            } catch (failure: Exception) {
                send(JSONObject().put("event", "speech.failed").put("error", JSONObject().put("code", "failed").put("message", failure.message ?: "write failed")))
            }
        }
        return JSONObject().put("job", job).put("path", path).put("output", file.absolutePath)
    }

    /** `clips.freeze`: the clip split at `time`, a hold of it laid between, the lane moved on. */
    private fun freeze(session: Session, request: JSONObject): String? = with(session) {
        val clip = clip(request.getString("clipId")) ?: throw Refusal("invalid", "no such clip")
        val kind = clip.optString("kind")
        if (kind != "video" && kind != "image") throw Refusal("invalid", "only a picture holds")
        val time = request.getDouble("time")
        val hold = request.optDouble("duration", 1.0).takeIf { !it.isNaN() && it > 0 } ?: 1.0
        val start = clip.getDouble("start")
        if (time <= start + MIN || time >= clip.end() - MIN) throw Refusal("invalid", "$time s is not inside the clip")
        val track = clip.getString("trackId")
        shift(track, time, hold, except = clip.getString("id"))
        val tail = JSONObject(clip.toString()).put("id", mint("c")).put("start", time + hold)
            .put("duration", clip.end() - time)
            .put("sourceStart", clip.getDouble("sourceStart") + (time - start) * clip.getDouble("speed"))
        val id = mint("c")
        val still = JSONObject(clip.toString()).put("id", id).put("start", time).put("duration", hold)
            .put("name", "Freeze").put("speed", 1.0)
        clip.put("duration", time - start)
        clips.put(still)
        clips.put(tail)
        id
    }

    /** `clips.replace`: the file imported, the clip pointed at it in place. */
    private fun replace(session: Session, request: JSONObject): String? = with(session) {
        val clip = clip(request.getString("clipId")) ?: throw Refusal("invalid", "no such clip")
        val file = request.getString("file")
        import(session, file)
        val media = media.objectsList().firstOrNull { it.getString("path") == file } ?: throw Refusal("failed", "$file would not import")
        clip.put("mediaId", media.getString("id")).put("name", media.optString("name"))
        null
    }

    /** `clips.reverse`: a job that pretends to write the copy. */
    private fun reverse(request: JSONObject): JSONObject {
        val session = session(request)
        val clip = session.clip(request.getString("clipId")) ?: throw Refusal("invalid", "no such clip")
        if (clip.optString("kind") !in setOf("video", "audio")) throw Refusal("invalid", "nothing to reverse")
        val job = "j${nextJob++}"
        val path = session.path
        val clipId = clip.getString("id")
        val sourceStart = clip.getDouble("sourceStart")
        val covered = clip.getDouble("duration") * clip.getDouble("speed")
        thread(name = "fake-reverse-$job") {
            fun send(event: JSONObject) = _events.tryEmit(event.put("job", job).put("path", path).put("clipId", clipId).toString())
            for (step in 1..20) {
                if (synchronized(this) { cancelled.remove(job) }) {
                    send(JSONObject().put("event", "reverse.failed").put("error", JSONObject().put("code", "cancelled").put("message", "Cancelled.")))
                    return@thread
                }
                send(JSONObject().put("event", "reverse.progress").put("fraction", step / 20.0))
                Thread.sleep(80)
            }
            send(JSONObject().put("event", "reverse.done").put("file", File(path, "cache/reversed-$clipId.mp4").absolutePath)
                .put("sourceStart", sourceStart).put("covered", covered))
        }
        return JSONObject().put("job", job).put("path", path).put("output", "")
    }

    /** A quiet hum, as a 16-bit mono WAV, standing in for a voice. */
    private fun writeTone(file: File, seconds: Double) {
        val rate = 16_000
        val samples = (seconds * rate).toInt()
        val data = java.nio.ByteBuffer.allocate(44 + samples * 2).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        data.put("RIFF".toByteArray()).putInt(36 + samples * 2).put("WAVE".toByteArray())
        data.put("fmt ".toByteArray()).putInt(16).putShort(1).putShort(1).putInt(rate).putInt(rate * 2).putShort(2).putShort(16)
        data.put("data".toByteArray()).putInt(samples * 2)
        for (i in 0 until samples) {
            val t = i.toDouble() / rate
            val envelope = 0.5 + 0.5 * kotlin.math.sin(2 * Math.PI * 3 * t)
            data.putShort((kotlin.math.sin(2 * Math.PI * 180 * t) * 3000 * envelope).toInt().toShort())
        }
        file.parentFile?.mkdirs()
        file.writeBytes(data.array())
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
        // A folder deleted behind the editor's back is no longer a recent.
        val all = JSONArray(recentsFile.readText())
        JSONArray().apply { all.objectsList().filter { File(it.optString("path"), DOCUMENT).exists() }.forEach { put(it) } }
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
        // Ids minted from here on must not be ones the document already holds
        // (the counter starts again with every process).
        val taken = (session.clips.objectsList() + session.media.objectsList() + session.tracks.objectsList())
            .mapNotNull { Regex("^[a-z]+(\\d+)$").find(it.optString("id"))?.groupValues?.get(1)?.toIntOrNull() }
        nextId = max(nextId, (taken.maxOrNull() ?: 0) + 1)
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
                // The lowest lane free for the whole run: lanes are stored bottom first.
                val track = named ?: (0 until tracks.length()).map { tracks.getJSONObject(it).getString("id") }
                    .firstOrNull { free(it, at, at + total) } ?: addTrack(session, top = true)
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
                    ?: firstFreeAbove(start, start + length)
                val id = mint("c")
                clips.put(JSONObject().put("id", id).put("trackId", track).put("mediaId", "")
                    .put("name", style.optString("content").lineSequence().firstOrNull().orEmpty())
                    .put("kind", "text").put("start", start).put("duration", length).put("sourceStart", 0.0)
                    .put("volume", 1.0).put("speed", 1.0).put("text", style))
                id
            }
            "addShapeClip" -> {
                val start = max(0.0, command.getDouble("start"))
                val length = command.optDouble("duration", 3.0).takeIf { !it.isNaN() } ?: 3.0
                val style = command.optJSONObject("style") ?: JSONObject().put("kind", "square")
                val track = command.optString("trackId").takeIf { !command.isNull("trackId") && it.isNotEmpty() }
                    ?: firstFreeAbove(start, start + length)
                val id = mint("c")
                clips.put(JSONObject().put("id", id).put("trackId", track).put("mediaId", "")
                    .put("name", command.optString("name").ifBlank { style.optString("kind") })
                    .put("kind", "shape").put("start", start).put("duration", length).put("sourceStart", 0.0)
                    .put("volume", 1.0).put("speed", 1.0).put("shape", style))
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
            "duplicateClips" -> {
                val ids = command.getJSONArray("clipIds").let { a -> (0 until a.length()).map { a.getString(it) } }
                var first: String? = null
                for (id in ids.mapNotNull { clip(it) }.sortedByDescending { it.getDouble("start") }) {
                    val copy = JSONObject(id.toString()).put("id", mint("c")).put("start", id.end())
                    clips.put(copy)
                    first = copy.getString("id")
                }
                first
            }
            "moveClips" -> {
                val known = tracks.objectsList().map { it.getString("id") }.toSet()
                val moves = command.getJSONArray("moves").objectsList()
                val moving = moves.map { it.getString("clipId") }.toSet()
                for (move in moves) {
                    val clip = clip(move.getString("clipId")) ?: continue
                    val track = move.optString("trackId").takeIf { it in known } ?: clip.getString("trackId")
                    var start = max(0.0, move.getDouble("start"))
                    val length = clip.getDouble("duration")
                    // Onto another clip is never where a move lands: the nearest gap that holds it.
                    val others = clipList().filter { it.getString("trackId") == track && it.getString("id") !in moving }
                        .map { it.getDouble("start") to it.end() }.sortedBy { it.first }
                    fun fits(at: Double) = others.none { (s, e) -> s < at + length - 1e-6 && e > at + 1e-6 }
                    if (!fits(start)) {
                        val candidates = listOf(0.0) + others.map { it.second } + others.map { it.first - length }
                        start = candidates.filter { it >= 0 && fits(it) }.minByOrNull { kotlin.math.abs(it - start) } ?: clip.getDouble("start")
                    }
                    clip.put("start", start).put("trackId", track)
                }
                null
            }
            "moveClipsToNewTrack" -> {
                // The lane and the move as one step; a move that lands no clip makes no lane.
                val moves = command.getJSONArray("moves").objectsList().filter { clip(it.getString("clipId")) != null }
                if (moves.isEmpty()) null
                else {
                    val lane = addTrack(session, top = !command.optBoolean("bottom"))
                    for (move in moves) clip(move.getString("clipId"))?.put("start", max(0.0, move.getDouble("start")))?.put("trackId", lane)
                    lane
                }
            }
            "addTrack" -> addTrack(session, top = !command.optBoolean("bottom"))
            "addTransitionClip" -> {
                val start = max(0.0, command.getDouble("start"))
                val length = command.optDouble("duration", 1.0).takeIf { !it.isNaN() } ?: 1.0
                val track = command.optString("trackId").takeIf { !command.isNull("trackId") && it.isNotEmpty() }
                    ?: firstFreeAbove(start, start + length)
                val id = mint("c")
                val pkg = command.getString("transitionId")
                clips.put(JSONObject().put("id", id).put("trackId", track).put("mediaId", "")
                    .put("name", command.optString("name").ifBlank { pkg }).put("kind", "transition")
                    .put("start", start).put("duration", length).put("sourceStart", 0.0).put("volume", 1.0).put("speed", 1.0)
                    .put("videoEffects", JSONArray().put(JSONObject().put("id", pkg).put("params", JSONObject()).put("enabled", true))))
                id
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
                if (patch.has("opacity")) clip.put("opacity", patch.getDouble("opacity").coerceIn(0.0, 1.0))
                for (flag in listOf("muted", "flipH", "flipV")) if (patch.has(flag)) clip.put(flag, patch.getBoolean(flag))
                for (fade in listOf("fadeIn", "fadeOut")) if (patch.has(fade)) clip.put(fade, max(0.0, patch.getDouble(fade)))
                for (chain in listOf("videoEffects", "filters")) {
                    if (patch.has(chain)) clip.put(chain, patch.optJSONArray(chain) ?: JSONArray())
                }
                // Absent leaves it alone, null takes it off, a value replaces it.
                for (field in listOf("transitionIn", "crop", "motion")) {
                    if (patch.has(field)) {
                        val value = patch.optJSONObject(field)
                        if (value == null) clip.remove(field) else clip.put(field, value)
                    }
                }
                if (patch.has("masks")) {
                    val masks = patch.optJSONArray("masks") ?: JSONArray()
                    // The engine mints an id for a mask that has none.
                    for (i in 0 until masks.length()) {
                        val mask = masks.getJSONObject(i)
                        if (mask.optString("id").isEmpty()) mask.put("id", mint("mask-"))
                    }
                    clip.put("masks", masks)
                }
                if (patch.has("text")) {
                    val text = patch.optJSONObject("text")
                    if (text == null) clip.remove("text") else {
                        clip.put("text", text).put("name", text.optString("content").lineSequence().firstOrNull().orEmpty())
                    }
                }
                null
            }
            "setClipTransform" -> {
                val clip = clip(command.getString("clipId")) ?: return null
                fun take(key: String) = command.optDouble(key).takeIf { command.has(key) && !command.isNull(key) && !it.isNaN() }
                take("scale")?.let { clip.put("scale", it.coerceIn(0.05, 8.0)) }
                take("offsetX")?.let { clip.put("offsetX", it.coerceIn(-3.0, 3.0)) }
                take("offsetY")?.let { clip.put("offsetY", it.coerceIn(-3.0, 3.0)) }
                take("rotation")?.let {
                    var r = it % 360.0
                    if (r > 180) r -= 360
                    if (r <= -180) r += 360
                    clip.put("rotation", r)
                }
                null
            }
            "setClipKey" -> {
                val clip = clip(command.getString("clipId")) ?: return null
                val property = command.getString("property")
                val at = command.getDouble("at").coerceIn(0.0, 1.0)
                val keys = (clip.optJSONArray("keys") ?: JSONArray()).objectsList()
                    .filterNot { it.optString("property") == property && kotlin.math.abs(it.optDouble("at") - at) < 0.002 }
                val added = keys + JSONObject().put("property", property).put("at", at).put("value", command.getDouble("value")).put("ease", "linear")
                clip.put("keys", JSONArray().apply { added.sortedWith(compareBy({ it.optString("property") }, { it.optDouble("at") })).forEach { put(it) } })
                null
            }
            "clearClipKey" -> {
                val clip = clip(command.getString("clipId")) ?: return null
                val property = command.getString("property")
                val at = command.getDouble("at")
                val kept = (clip.optJSONArray("keys") ?: JSONArray()).objectsList()
                    .filterNot { it.optString("property") == property && kotlin.math.abs(it.optDouble("at") - at) < 0.002 }
                clip.put("keys", JSONArray().apply { kept.forEach { put(it) } })
                null
            }
            "setCaptionLook" -> {
                val trackId = command.getString("trackId")
                val look = command.optJSONObject("look")
                val lane = tracks.objectsList().firstOrNull { it.optString("id") == trackId }
                if (lane != null) {
                    if (look == null) lane.remove("captions") else lane.put("captions", JSONObject(look.toString()))
                    if (look != null) {
                        for (clip in clipList().filter { it.optString("trackId") == trackId && it.optString("kind") == "text" }) {
                            val old = clip.optJSONObject("text") ?: JSONObject()
                            val style = JSONObject(look.optJSONObject("style")?.toString() ?: "{}")
                                .put("content", old.optString("content")).apply { old.optJSONArray("runs")?.let { put("runs", it) } }
                            clip.put("text", style).put("offsetY", look.optDouble("offsetY", 0.0))
                        }
                    }
                }
                null
            }
            "setCover" -> {
                val cover = command.optJSONObject("cover")
                if (cover == null) session.project.remove("cover")
                else session.project.put("cover", JSONObject(cover.toString()).apply { if (has("time")) put("time", max(0.0, getDouble("time"))) })
                null
            }
            "setTimelineBackground" -> {
                timeline.put("background", command.getJSONObject("background"))
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
        // Stored bottom first, as the engine does: the last lane is drawn on top.
        if (top) list.add(track) else list.add(0, track)
        session.timeline.put("tracks", JSONArray().apply { list.forEach { put(it) } })
        return id
    }

    /** The first lane free over the span above the highest one taken there, or a new one on top: where a title goes. */
    private fun Session.firstFreeAbove(start: Double, end: Double): String {
        val ids = tracks.objectsList().map { it.getString("id") }
        val floor = ids.indices.filter { !free(ids[it], start, end) }.maxOfOrNull { it + 1 } ?: 0
        return ids.drop(floor).firstOrNull { free(it, start, end) } ?: addTrack(this, top = true)
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
        private val FAKE_WORDS = listOf(
            "This is what the in-memory engine heard.",
            "Every line can be corrected before it is placed.",
            "The real engine hears the clip's own sound.",
            "Captions land over the clip as titles.",
        )
    }
}

/** A handful of packages shaped as `catalogue.list` gives them, for the sheets to show without the engine. */
private object FakeCatalogue {
    private fun param(key: String, label: String, min: Double, max: Double, default: Double, step: Double = 0.0, unit: String = "", type: String = "float") =
        JSONObject().put("key", key).put("label", label).put("type", type).put("min", min).put("max", max).put("default", default)
            .put("step", step).put("unit", unit).put("animate", true).put("values", JSONArray()).put("labels", JSONArray())

    private fun pkg(id: String, name: String, kind: String, category: String, description: String, vararg params: JSONObject) =
        JSONObject().put("id", id).put("name", name).put("kind", kind).put("category", category).put("description", description)
            .put("intensity", if (params.isNotEmpty()) params[0].getString("key") else JSONObject.NULL)
            .put("params", JSONArray().apply { params.forEach { put(it) } })

    private val all: List<JSONObject> by lazy {
        listOf(
            pkg("concat.blur", "Blur", "effect", "Blur", "Softens the picture.", param("radius", "Radius", 0.0, 50.0, 8.0, 1.0, "px")),
            pkg("concat.zoom-blur", "Zoom blur", "effect", "Blur", "Streaks out from the centre.", param("amount", "Amount", 0.0, 1.0, 0.4)),
            pkg("concat.glow", "Glow", "effect", "Light", "A bloom around bright parts.", param("strength", "Strength", 0.0, 2.0, 0.8), param("threshold", "Threshold", 0.0, 1.0, 0.6)),
            pkg("concat.vignette", "Vignette", "effect", "Light", "Darkens the corners.", param("amount", "Amount", 0.0, 1.0, 0.5), param("softness", "Softness", 0.0, 1.0, 0.5)),
            pkg("concat.shake", "Shake", "effect", "Motion", "A hand-held wobble.", param("strength", "Strength", 0.0, 1.0, 0.3), param("speed", "Speed", 0.1, 10.0, 2.0, 0.1, "Hz")),
            pkg("concat.pixelate", "Pixelate", "effect", "Stylize", "Big square pixels.", param("size", "Size", 1.0, 100.0, 12.0, 1.0, "px", "int")),
            pkg("concat.warm", "Warm", "filter", "Colour", "A golden cast.", param("intensity", "Intensity", 0.0, 1.0, 0.7)),
            pkg("concat.cool", "Cool", "filter", "Colour", "A blue cast.", param("intensity", "Intensity", 0.0, 1.0, 0.7)),
            pkg("concat.mono", "Mono", "filter", "Black and white", "No colour.", param("intensity", "Intensity", 0.0, 1.0, 1.0)),
            pkg("concat.noir", "Noir", "filter", "Black and white", "Hard contrast, no colour.", param("intensity", "Intensity", 0.0, 1.0, 1.0)),
            pkg("concat.vintage", "Vintage", "filter", "Film", "Faded, warm film.", param("intensity", "Intensity", 0.0, 1.0, 0.8)),
            pkg("concat.echo", "Echo", "audio", "Space", "Repeats the sound.", param("delay", "Delay", 0.05, 1.0, 0.3, 0.01, "s"), param("feedback", "Feedback", 0.0, 0.9, 0.4)),
            pkg("concat.reverb", "Reverb", "audio", "Space", "A room around the sound.", param("mix", "Mix", 0.0, 1.0, 0.3)),
            pkg("concat.cross-fade", "Cross fade", "transition", "Basic", "One picture dissolves into the next."),
            pkg("concat.dip-black", "Dip to black", "transition", "Basic", "Down to black and back up."),
            pkg("concat.push", "Push", "transition", "Slide", "The next shot pushes this one out.", param("angle", "Direction", 0.0, 360.0, 0.0, 90.0, "°")),
            pkg("concat.wipe", "Wipe", "transition", "Slide", "A line sweeps across.", param("softness", "Softness", 0.0, 1.0, 0.1)),
            pkg("concat.zoom", "Zoom", "transition", "Zoom", "Punches into the next shot.", param("amount", "Amount", 0.0, 1.0, 0.5)),
        )
    }

    fun list(kind: String?): JSONArray = JSONArray().apply {
        all.filter { kind == null || it.getString("kind") == kind }.sortedBy { it.getString("id") }.forEach { put(JSONObject(it.toString())) }
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

    private fun paint(surface: Surface) {
        val canvas: Canvas = try {
            surface.lockCanvas(null)
        } catch (_: Exception) {
            return
        }
        try {
            FakeFrame.paint(canvas, view, position(), showClock = true)
        } finally {
            try {
                surface.unlockCanvasAndPost(canvas)
            } catch (_: Exception) {
            }
        }
    }
}

/**
 * The in-memory engine's picture at an instant, on any canvas: the
 * preview's Surface and `preview.frame`'s PNG. Pictures are gradients with
 * their names; titles are drawn where their transform puts them.
 */
internal object FakeFrame {
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG)
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { textAlign = Paint.Align.CENTER }

    @Synchronized
    fun paint(canvas: Canvas, view: JSONObject?, at: Double, showClock: Boolean) {
        canvas.drawColor(Color.rgb(14, 14, 16))
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
        val background = timeline?.optJSONObject("background")
        fill.shader = null
        fill.color = when (background?.optString("kind")) {
            "color" -> try {
                Color.parseColor(background.optString("color"))
            } catch (_: Exception) {
                Color.BLACK
            }
            "blur" -> Color.rgb(40, 44, 52)
            else -> Color.BLACK
        }
        canvas.drawRect(frame, fill)
        val clips = timeline?.optJSONArray("clips")?.objectsList().orEmpty()
            .filter { at >= it.optDouble("start") && at < it.optDouble("start") + it.optDouble("duration") }
        val tracks = timeline?.optJSONArray("tracks")?.objectsList().orEmpty().map { it.optString("id") }
        canvas.save()
        canvas.clipRect(frame)
        // Lanes are stored bottom first: lower lanes are drawn first, under the rest.
        for (clip in clips.sortedBy { tracks.indexOf(it.optString("trackId")) }) {
            val kind = clip.optString("kind")
            if (kind != "video" && kind != "image" && kind != "text") continue
            val scale = clip.optDouble("scale", 1.0).toFloat()
            val cx = frame.centerX() + clip.optDouble("offsetX", 0.0).toFloat() * frame.width()
            val cy = frame.centerY() + clip.optDouble("offsetY", 0.0).toFloat() * frame.height()
            val opacity = (clip.optDouble("opacity", 1.0) * 255).toInt().coerceIn(0, 255)
            canvas.save()
            canvas.rotate(clip.optDouble("rotation", 0.0).toFloat(), cx, cy)
            if (kind == "text") {
                val style = clip.optJSONObject("text") ?: JSONObject()
                text.textSize = (style.optDouble("fontSize", 0.09) * frame.height() * scale).toFloat().coerceAtLeast(4f)
                text.typeface = if (style.optDouble("fontWeight", 700.0) >= 600) android.graphics.Typeface.DEFAULT_BOLD else android.graphics.Typeface.DEFAULT
                text.color = try {
                    Color.parseColor(style.optString("color", "#ffffff"))
                } catch (_: Exception) {
                    Color.WHITE
                }
                text.alpha = opacity
                text.setShadowLayer(text.textSize * 0.08f, 0f, text.textSize * 0.04f, Color.argb(160, 0, 0, 0))
                val lines = style.optString("content").lines()
                val lineHeight = text.textSize * 1.2f
                var y = cy - lineHeight * (lines.size - 1) / 2f + text.textSize * 0.35f
                for (line in lines) {
                    canvas.drawText(line, cx, y, text)
                    y += lineHeight
                }
                text.clearShadowLayer()
            } else {
                val hue = (clip.optString("mediaId").hashCode() and 0xff) * 360f / 256f
                val bw = frame.width() * scale
                val bh = frame.height() * scale
                val box = RectF(cx - bw / 2, cy - bh / 2, cx + bw / 2, cy + bh / 2)
                fill.shader = android.graphics.LinearGradient(
                    box.left, box.top, box.right, box.bottom,
                    Color.HSVToColor(floatArrayOf(hue, 0.55f, 0.6f)),
                    Color.HSVToColor(floatArrayOf((hue + 50f) % 360f, 0.6f, 0.28f)),
                    android.graphics.Shader.TileMode.CLAMP,
                )
                fill.alpha = opacity
                canvas.drawRect(box, fill)
                fill.shader = null
                fill.alpha = 255
                text.textSize = frame.height() * 0.028f
                text.color = Color.argb(150, 255, 255, 255)
                text.typeface = android.graphics.Typeface.DEFAULT
                canvas.drawText(clip.optString("name"), cx, box.bottom - text.textSize * 2, text)
            }
            canvas.restore()
        }
        canvas.restore()
        if (showClock) {
            text.textSize = frame.height() * 0.022f
            text.color = Color.argb(120, 255, 255, 255)
            text.typeface = android.graphics.Typeface.DEFAULT
            canvas.drawText("in-memory engine  %.2f s".format(at), frame.centerX(), frame.top + text.textSize * 2, text)
        }
    }
}

/** Stand-ins for the engine's filmstrips and sound outlines: gradient tiles with their frame number, and a made-up wave. */
internal object FakePictures {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val label = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(210, 255, 255, 255)
        typeface = android.graphics.Typeface.DEFAULT_BOLD
    }

    @Synchronized
    fun filmstrip(request: JSONObject): JSONObject {
        val file = request.getString("file")
        val count = request.optInt("count", 8).coerceIn(1, 60)
        val height = request.optInt("height", 96).coerceIn(16, 240)
        val from = request.optDouble("from", 0.0).takeIf { !it.isNaN() } ?: 0.0
        val to = request.optDouble("to", from + 5.0).takeIf { !it.isNaN() } ?: (from + 5.0)
        val tile = height
        val bitmap = android.graphics.Bitmap.createBitmap(tile * count, height, android.graphics.Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val base = (File(file).name.hashCode() and 0xff) * 360f / 256f
        for (i in 0 until count) {
            val t = from + (i + 0.5) * (to - from) / count
            val hue = (base + t.toFloat() * 9f) % 360f
            val left = (i * tile).toFloat()
            paint.shader = android.graphics.LinearGradient(
                left, 0f, left + tile, height.toFloat(),
                Color.HSVToColor(floatArrayOf(hue, 0.55f, 0.75f)),
                Color.HSVToColor(floatArrayOf((hue + 40f) % 360f, 0.7f, 0.32f)),
                android.graphics.Shader.TileMode.CLAMP,
            )
            canvas.drawRect(left, 0f, left + tile, height.toFloat(), paint)
            paint.shader = null
            // A horizon and a sun, so the tiles read as pictures of something.
            paint.color = Color.argb(60, 0, 0, 0)
            canvas.drawRect(left, height * 0.68f, left + tile, height.toFloat(), paint)
            paint.color = Color.argb(110, 255, 255, 255)
            canvas.drawCircle(left + tile * (0.3f + 0.4f * ((t.toFloat() * 0.13f) % 1f)), height * 0.36f, tile * 0.1f, paint)
            paint.color = Color.argb(80, 0, 0, 0)
            canvas.drawRect(left + tile - 1f, 0f, left + tile.toFloat(), height.toFloat(), paint)
            label.textSize = height * 0.16f
            canvas.drawText("${(t * 30).toInt()}", left + height * 0.08f, height * 0.92f, label)
        }
        val bytes = java.io.ByteArrayOutputStream().also { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        return JSONObject().put("width", bitmap.width).put("height", bitmap.height)
            .put("png", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP))
    }

    fun peaks(request: JSONObject, duration: Double): JSONObject {
        val perSecond = request.optDouble("perSecond", 20.0).takeIf { !it.isNaN() && it > 0 } ?: 20.0
        val n = max(1, (duration * perSecond).toInt())
        val seed = request.getString("file").hashCode()
        val random = java.util.Random(seed.toLong())
        val low = JSONArray()
        val high = JSONArray()
        for (i in 0 until n) {
            val t = i / perSecond
            val phrase = 0.5 + 0.5 * kotlin.math.sin(t * 1.7 + seed % 7)
            val beat = 0.6 + 0.4 * kotlin.math.abs(kotlin.math.sin(t * Math.PI * 2))
            val level = (0.12 + 0.75 * phrase * beat * (0.7 + 0.3 * random.nextDouble())).coerceIn(0.0, 1.0)
            high.put(level)
            low.put(-level * (0.85 + 0.1 * random.nextDouble()))
        }
        return JSONObject().put("min", low).put("max", high).put("perSecond", perSecond)
    }
}
