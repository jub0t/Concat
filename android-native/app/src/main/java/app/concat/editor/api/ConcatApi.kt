// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.api

import android.util.Log
import app.concat.editor.engine.EngineBridge
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/** A file's sound outline: `media.peaks`. */
class Peaks(val min: FloatArray, val max: FloatArray, val perSecond: Float)

/** A refusal from the API: its `code` ("refused", "notOpen", ...) and the sentence to show. */
class ApiException(val code: String, override val message: String) : Exception(message)

/**
 * The API's requests as Kotlin calls, each one JSON object through
 * [EngineBridge.call] on the IO dispatcher. The request shapes are
 * concat-api's `Request` (tagged by `method`, camelCase fields), and an
 * edit is concat-project's `Command` (tagged by `op`), as [Commands]
 * builds them.
 */
class ConcatApi(val engine: EngineBridge) {
    val events: Flow<ApiEvent> = engine.events.mapNotNull { ApiEvent.parse(it) }

    /** Every event as its JSON object: captions, models and speech read these. */
    val rawEvents: Flow<JSONObject> = engine.events.mapNotNull {
        try {
            JSONObject(it)
        } catch (_: Exception) {
            null
        }
    }

    /** Sends one request; its `result`, or the error thrown. */
    suspend fun request(request: JSONObject): Any = withContext(Dispatchers.IO) {
        val text = request.toString()
        val reply = JSONObject(engine.call(text))
        reply.optJSONObject("error")?.let { error ->
            Log.w(TAG, "${request.optString("method")} refused: $error")
            throw ApiException(error.optString("code", "failed"), error.optString("message", "the editor said no"))
        }
        reply.opt("result") ?: JSONObject()
    }

    private suspend fun view(request: JSONObject) = ProjectView.from(request(request) as JSONObject)

    suspend fun listProjects(): List<ProjectInfo> =
        (request(method("project.list")) as? JSONArray)?.objects()?.map(ProjectInfo::from) ?: emptyList()

    /** Makes a project folder named after [name] under [location], opens it and says where it is. */
    suspend fun createProject(location: String, name: String, width: Int, height: Int, fps: Int): String {
        request(
            method("project.create")
                .put("location", location)
                .put("name", name)
                .put("video", JSONObject().put("width", width).put("height", height).put("rateNum", fps).put("rateDen", 1)),
        )
        // The reply is the editor view, which names no folder; recents put
        // the new project first.
        return listProjects().firstOrNull()?.path ?: throw ApiException("failed", "the new project is not in recents")
    }

    suspend fun open(path: String) = view(method("project.open").put("path", path))
    suspend fun get(path: String) = view(method("project.get").put("path", path))
    suspend fun save(path: String) = request(method("project.save").put("path", path))

    /** Writes the document under a new name: `project.save` with `name`. */
    suspend fun rename(path: String, name: String) = request(method("project.save").put("path", path).put("name", name))

    /** Sets the active timeline's frame and rate, one undo step: `project.setVideo`. */
    suspend fun setVideo(path: String, width: Int, height: Int, rateNum: Long, rateDen: Long) = view(
        method("project.setVideo").put("path", path)
            .put("video", JSONObject().put("width", width).put("height", height).put("rateNum", rateNum).put("rateDen", rateDen)),
    )

    // -- pictures and sound outlines for the timeline --
    //
    // Plain JSON requests like every other, so both bridges carry them
    // without a binding of their own.

    /**
     * `media.filmstrip`: [count] pictures of [file] side by side at
     * [height] pixels, evenly across [from]..[to] seconds of it, as PNG
     * bytes. The reply's picture is `png` (concat-api's `Picture`); an
     * older build's `pngBase64` is read too.
     */
    suspend fun mediaFilmstrip(file: String, from: Double, to: Double, count: Int, height: Int): ByteArray {
        val reply = request(
            method("media.filmstrip").put("file", file).put("from", from).put("to", to)
                .put("count", count.coerceIn(1, 60)).put("height", height.coerceIn(16, 240)),
        ) as JSONObject
        return decodePicture(reply)
    }

    /** `media.peaks`: the loudness outline of [file]'s sound, [perSecond] slices a second. */
    suspend fun mediaPeaks(path: String?, file: String, perSecond: Int = 20): Peaks {
        val reply = request(
            method("media.peaks").put("path", path ?: JSONObject.NULL).put("file", file)
                .put("stream", JSONObject.NULL).put("perSecond", perSecond),
        ) as JSONObject
        fun floats(name: String): FloatArray {
            val array = reply.optJSONArray(name) ?: return FloatArray(0)
            return FloatArray(array.length()) { array.optDouble(it).toFloat() }
        }
        return Peaks(floats("min"), floats("max"), reply.optDouble("perSecond", perSecond.toDouble()).toFloat())
    }

    /** `preview.frame`: the true frame at [time], written to [output] as a PNG. */
    suspend fun previewFrameTo(path: String, time: Double, output: String, width: Int, height: Int) = request(
        method("preview.frame").put("path", path).put("time", time).put("output", output).put("width", width).put("height", height),
    )

    private fun decodePicture(reply: JSONObject): ByteArray {
        val text = reply.optString("png").ifEmpty { reply.optString("pngBase64") }
        if (text.isEmpty()) throw ApiException("failed", "no picture in the reply")
        return android.util.Base64.decode(text, android.util.Base64.DEFAULT)
    }
    suspend fun close(path: String, save: Boolean) = request(method("project.close").put("path", path).put("save", save))
    suspend fun undo(path: String) = view(method("edit.undo").put("path", path))
    suspend fun redo(path: String) = view(method("edit.redo").put("path", path))
    suspend fun import(path: String, file: String) = view(method("media.import").put("path", path).put("file", file))

    suspend fun apply(path: String, command: JSONObject) =
        view(method("edit.apply").put("path", path).put("command", command))

    /** Starts an export; the job's name. */
    suspend fun exportRun(path: String, output: String, width: Int, height: Int, crf: Int, preset: String): String {
        val started = request(
            method("export.run")
                .put("path", path)
                .put("output", output)
                .put("width", width)
                .put("height", height)
                .put("crf", crf)
                .put("preset", preset)
                .put("codec", "h264"),
        ) as JSONObject
        return started.getString("job")
    }

    suspend fun exportCancel(job: String) = request(method("export.cancel").put("job", job))

    suspend fun version(): VersionInfo {
        val json = request(method("version")) as JSONObject
        val caps = json.optJSONArray("capabilities")
        return VersionInfo(
            json.optString("apiVersion"),
            json.optString("concat"),
            caps?.let { a -> (0 until a.length()).map { a.optString(it) } } ?: emptyList(),
        )
    }

    /** The packages of one kind ("effect", "filter", "audio", "transition"), or all. */
    suspend fun catalogue(kind: String? = null): List<PackageInfo> {
        val request = method("catalogue.list").apply { if (kind != null) put("kind", kind) }
        return (request(request) as? JSONArray)?.objects()?.map(PackageInfo::from) ?: emptyList()
    }

    /** `project.cover`: the cover written to [output] as a PNG of [width] × [height]. */
    suspend fun projectCover(path: String, output: String, width: Int, height: Int) = request(
        method("project.cover").put("path", path).put("output", output).put("width", width).put("height", height),
    )

    // -- stickers and bounds --

    /** `shapes.list`: the Stickers page's cards, in order. */
    suspend fun shapes(): List<ShapeCard> =
        (request(method("shapes.list")) as? JSONArray)?.objects()?.map { ShapeCard(it.getString("id"), it.getJSONObject("style")) }
            ?: emptyList()

    /** `clips.bounds`: each title's and shape's painted block, by clip id. */
    suspend fun bounds(path: String): Map<String, ClipBounds> =
        (request(method("clips.bounds").put("path", path)) as? JSONArray)?.objects()?.associate {
            it.getString("clipId") to ClipBounds(it.optDouble("width"), it.optDouble("height"), it.optDouble("offsetX", 0.0), it.optDouble("offsetY", 0.0))
        } ?: emptyMap()

    // -- speech models --

    suspend fun models(): ModelsInfo = ModelsInfo.from(request(method("models.list")) as JSONObject)

    /** Starts a download; the job's name. [kind] is "transcriber" or "speech". */
    suspend fun modelsDownload(kind: String, id: String): String =
        (request(method("models.download").put("kind", kind).put("id", id)) as JSONObject).getString("job")

    suspend fun modelsDelete(kind: String, id: String) = request(method("models.delete").put("kind", kind).put("id", id))

    // -- captions --

    /** Starts hearing [clipId]; the job's name. Its words come in `captions.done`. */
    suspend fun captionsTranscribe(path: String, clipId: String, model: String, language: String?): String {
        val request = method("captions.transcribe").put("path", path).put("clipId", clipId).put("model", model)
        if (!language.isNullOrBlank()) request.put("language", language)
        return (request(request) as JSONObject).getString("job")
    }

    /** Puts [segments] over [clipId] as titles, one undo step. */
    suspend fun captionsPlace(path: String, clipId: String, segments: List<CaptionSegment>, placement: String, size: String) = view(
        method("captions.place").put("path", path).put("clipId", clipId)
            .put("segments", JSONArray().apply { segments.forEach { put(it.toJson()) } })
            .put("placement", placement).put("size", size),
    )

    suspend fun captionsCancel(job: String) = request(method("captions.cancel").put("job", job))

    // -- reading aloud --

    /** Starts reading [text] into a file in the project; the job's name. */
    suspend fun speechSpeak(path: String, text: String, model: String, voice: Int, speed: Double): String =
        (request(
            method("speech.speak").put("path", path).put("text", text).put("model", model)
                .put("voice", voice).put("speed", speed),
        ) as JSONObject).getString("job")

    suspend fun speechCancel(job: String) = request(method("speech.cancel").put("job", job))

    // -- clip operations that need the host: a frame grabbed, a file probed or written --

    /** The picture at [time] held for [duration] seconds: split, hold, ripple, one undo step. */
    suspend fun clipsFreeze(path: String, clipId: String, time: Double, duration: Double = 1.0) = view(
        method("clips.freeze").put("path", path).put("clipId", clipId).put("time", time).put("duration", duration),
    )

    /** The clip pointed at [file], keeping its place, length and look. */
    suspend fun clipsReplace(path: String, clipId: String, file: String) = view(
        method("clips.replace").put("path", path).put("clipId", clipId).put("file", file),
    )

    /** Starts writing the clip's span backwards; the job's name. `reverse.done` then says where. */
    suspend fun clipsReverse(path: String, clipId: String): String =
        (request(method("clips.reverse").put("path", path).put("clipId", clipId)) as JSONObject).getString("job")

    /** The clip shown from the copy a `reverse.done` [event] named. */
    suspend fun clipsUseReversed(path: String, event: JSONObject) = view(
        method("clips.useReversed").put("path", path).put("clipId", event.getString("clipId"))
            .put("file", event.getString("file")).put("sourceStart", event.getDouble("sourceStart"))
            .put("covered", event.getDouble("covered")),
    )

    suspend fun reverseCancel(job: String) = request(method("reverse.cancel").put("job", job))

    private fun method(name: String) = JSONObject().put("method", name)

    private companion object {
        const val TAG = "concat-api"
    }
}

/** concat-project's `Command`s, as `edit.apply` carries them: `{"op": ..., camelCase fields}`. */
object Commands {
    private fun op(name: String) = JSONObject().put("op", name)
    private fun ids(ids: List<String>) = JSONArray().apply { ids.forEach { put(it) } }

    fun addClipsInSequence(mediaIds: List<String>, start: Double, trackId: String? = null) =
        op("addClipsInSequence").put("mediaIds", ids(mediaIds)).put("start", start)
            .put("trackId", trackId ?: JSONObject.NULL).put("ripple", trackId != null)

    fun addClip(mediaId: String, trackId: String, start: Double, ripple: Boolean) =
        op("addClip").put("mediaId", mediaId).put("trackId", trackId).put("start", start).put("ripple", ripple)

    fun addTextClip(start: Double, style: JSONObject, duration: Double? = null, trackId: String? = null) =
        op("addTextClip").put("trackId", trackId ?: JSONObject.NULL).put("above", true).put("start", start)
            .put("style", style).apply { if (duration != null) put("duration", duration) }

    /** A caption lane's look ([look] is `{style, offsetY}`), every caption on it restyled to match, words kept. */
    fun setCaptionLook(trackId: String, look: JSONObject) = op("setCaptionLook").put("trackId", trackId).put("look", look)

    /** The project's cover: a frame at [time], a picture at [picture], or with neither back to a frame early in. */
    fun setCover(time: Double? = null, picture: String? = null) = op("setCover").put(
        "cover",
        when {
            picture != null -> JSONObject().put("kind", "picture").put("path", picture)
            time != null -> JSONObject().put("kind", "frame").put("time", time)
            else -> JSONObject.NULL
        },
    )

    /** A sticker over the picture, on the first free lane above it: [style] is the card's, [name] what the lane calls it. */
    fun addShapeClip(start: Double, style: JSONObject, name: String, duration: Double? = null) =
        op("addShapeClip").put("trackId", JSONObject.NULL).put("above", true).put("start", start)
            .put("style", style).put("name", name).apply { if (duration != null) put("duration", duration) }

    fun splitClips(clipIds: List<String>, time: Double) = op("splitClips").put("clipIds", ids(clipIds)).put("time", time)

    fun duplicateClips(clipIds: List<String>) = op("duplicateClips").put("clipIds", ids(clipIds))

    fun removeClips(clipIds: List<String>, ripple: Boolean = true) =
        op("removeClips").put("clipIds", ids(clipIds)).put("ripple", ripple)

    fun setClipSpeed(clipId: String, speed: Double) =
        op("setClipSpeed").put("clipId", clipId).put("speed", speed).put("ripple", true)

    fun trimClip(clipId: String, edge: String, delta: Double, ripple: Boolean) =
        op("trimClip").put("clipId", clipId).put("edge", edge).put("delta", delta).put("ripple", ripple)

    fun updateClip(clipId: String, patch: JSONObject) = op("updateClip").put("clipId", clipId).put("patch", patch)

    /**
     * The picture's placement: concat-project's `SetClipTransform` (the
     * transform is not in `ClipPatch`). Scale over the fitted size,
     * offsets as fractions of the frame (positive right and down),
     * clockwise degrees.
     */
    fun setClipTransform(clipId: String, scale: Double, offsetX: Double, offsetY: Double, rotation: Double) =
        op("setClipTransform").put("clipId", clipId).put("scale", scale).put("offsetX", offsetX)
            .put("offsetY", offsetY).put("rotation", rotation)

    /** One key on one property ("scale", "offsetX", "offsetY", "rotation", "opacity", "volume"), [at] 0..1 into the clip. */
    fun setClipKey(clipId: String, property: String, at: Double, value: Double) =
        op("setClipKey").put("clipId", clipId).put("property", property).put("at", at).put("value", value)

    fun clearClipKey(clipId: String, property: String, at: Double) =
        op("clearClipKey").put("clipId", clipId).put("property", property).put("at", at)

    /** What fills the frame behind the lowest picture: `{"kind":"none"}`, `{"kind":"blur","amount":..}`, `{"kind":"color","color":"#rrggbb"}`. */
    fun setTimelineBackground(timelineId: String, background: JSONObject) =
        op("setTimelineBackground").put("timelineId", timelineId).put("background", background)

    /** Clips to new starts and lanes in one undo step: `{"op":"moveClips","moves":[{clipId,start,trackId}]}`. */
    fun moveClips(moves: List<Triple<String, Double, String>>) = op("moveClips").put(
        "moves",
        JSONArray().apply {
            moves.forEach { (clip, start, track) -> put(JSONObject().put("clipId", clip).put("start", start).put("trackId", track)) }
        },
    )

    /** Clips onto a lane made for them, on top unless [bottom]: the lane and the move are one undo step. */
    fun moveClipsToNewTrack(moves: List<Pair<String, Double>>, bottom: Boolean) = op("moveClipsToNewTrack").put(
        "moves",
        JSONArray().apply { moves.forEach { (clip, start) -> put(JSONObject().put("clipId", clip).put("start", start)) } },
    ).put("bottom", bottom)

    /** A new lane, on top of the others unless [bottom]. */
    fun addTrack(bottom: Boolean) = op("addTrack").put("bottom", bottom)

    /** A transition as a clip of its own over the lanes beneath it. */
    fun addTransitionClip(transitionId: String, name: String, start: Double, duration: Double? = null, trackId: String? = null) =
        op("addTransitionClip").put("transitionId", transitionId).put("name", name).put("start", start)
            .put("trackId", trackId ?: JSONObject.NULL).apply { if (duration != null) put("duration", duration) }

    /** A whole chain as `updateClip` replaces it: [field] is "videoEffects" or "filters". */
    fun setChain(clipId: String, field: String, chain: List<JSONObject>) =
        updateClip(clipId, JSONObject().put(field, JSONArray().apply { chain.forEach { put(it) } }))

    fun batch(commands: List<JSONObject>) = op("batch").put("commands", JSONArray().apply { commands.forEach { put(it) } })

    /** A title's style: the document's `TextStyle`; fields left out take its defaults. */
    fun textStyle(content: String, fontSize: Double, color: String, base: JSONObject? = null, fontWeight: Double? = null): JSONObject =
        JSONObject(base?.toString() ?: "{}").put("content", content).put("fontSize", fontSize).put("color", color)
            .apply { if (fontWeight != null) put("fontWeight", fontWeight) }
}
