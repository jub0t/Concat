// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.api

import android.util.Log
import app.concat.editor.spike.engine.EngineBridge
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

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

    fun splitClips(clipIds: List<String>, time: Double) = op("splitClips").put("clipIds", ids(clipIds)).put("time", time)

    fun duplicateClips(clipIds: List<String>) = op("duplicateClips").put("clipIds", ids(clipIds))

    fun removeClips(clipIds: List<String>, ripple: Boolean = true) =
        op("removeClips").put("clipIds", ids(clipIds)).put("ripple", ripple)

    fun setClipSpeed(clipId: String, speed: Double) =
        op("setClipSpeed").put("clipId", clipId).put("speed", speed).put("ripple", true)

    fun trimClip(clipId: String, edge: String, delta: Double, ripple: Boolean) =
        op("trimClip").put("clipId", clipId).put("edge", edge).put("delta", delta).put("ripple", ripple)

    fun updateClip(clipId: String, patch: JSONObject) = op("updateClip").put("clipId", clipId).put("patch", patch)

    fun batch(commands: List<JSONObject>) = op("batch").put("commands", JSONArray().apply { commands.forEach { put(it) } })

    /** A title's style: the document's `TextStyle`; fields left out take its defaults. */
    fun textStyle(content: String, fontSize: Double, color: String, base: JSONObject? = null): JSONObject =
        JSONObject(base?.toString() ?: "{}").put("content", content).put("fontSize", fontSize).put("color", color)
}
