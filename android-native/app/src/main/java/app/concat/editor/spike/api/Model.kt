// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.api

import org.json.JSONArray
import org.json.JSONObject

/*
 * What the app reads out of the API's replies (concat-api's message.rs,
 * concat-host's EditorView and ProjectInfo, concat-project's model). Only
 * the fields the phone shows; the rest of each object is ignored, and kept
 * where a command needs it back whole (a title's style).
 */

/** One recents entry: `project.list`. */
data class ProjectInfo(
    val path: String,
    val name: String,
    val width: Int,
    val height: Int,
    val rateNum: Long,
    val rateDen: Long,
    val openedAt: Long,
) {
    companion object {
        fun from(json: JSONObject) = ProjectInfo(
            path = json.optString("path"),
            name = json.optString("name"),
            width = json.optInt("width"),
            height = json.optInt("height"),
            rateNum = json.optLong("rateNum", 30),
            rateDen = json.optLong("rateDen", 1).coerceAtLeast(1),
            openedAt = json.optLong("openedAt"),
        )
    }
}

enum class ClipKind { Video, Audio, Image, Text, Layer, Other }

data class MediaItem(val id: String, val path: String, val name: String, val kind: String, val duration: Double?)

data class Track(val id: String, val visible: Boolean, val muted: Boolean)

data class Clip(
    val id: String,
    val trackId: String,
    val mediaId: String,
    val name: String,
    val kind: ClipKind,
    val start: Double,
    val duration: Double,
    val sourceStart: Double,
    val volume: Double,
    val speed: Double,
    /** A title's whole style, as the document holds it. */
    val text: JSONObject?,
) {
    val end get() = start + duration
}

/** `EditorView`: what every project and edit method replies with. */
data class ProjectView(
    val name: String,
    val width: Int,
    val height: Int,
    val rateNum: Long,
    val rateDen: Long,
    val canUndo: Boolean,
    val canRedo: Boolean,
    val media: List<MediaItem>,
    /** The active timeline's lanes, top to bottom. */
    val tracks: List<Track>,
    val clips: List<Clip>,
    val createdId: String?,
) {
    val fps: Double get() = rateNum.toDouble() / rateDen.coerceAtLeast(1)
    val duration: Double get() = clips.maxOfOrNull { it.end } ?: 0.0

    /** Where the picture lanes end: where an import lands. */
    val pictureEnd: Double
        get() = clips.filter { it.kind == ClipKind.Video || it.kind == ClipKind.Image }.maxOfOrNull { it.end } ?: 0.0

    fun clip(id: String?) = clips.firstOrNull { it.id == id }
    fun mediaOf(clip: Clip) = media.firstOrNull { it.id == clip.mediaId }

    companion object {
        fun from(json: JSONObject): ProjectView {
            val project = json.optJSONObject("project") ?: JSONObject()
            val settings = json.optJSONObject("settings") ?: JSONObject()
            val timelines = project.optJSONArray("timelines") ?: JSONArray()
            val activeId = project.optString("activeTimelineId")
            var timeline: JSONObject? = null
            for (i in 0 until timelines.length()) {
                val candidate = timelines.getJSONObject(i)
                if (timeline == null || candidate.optString("id") == activeId) timeline = candidate
            }
            timeline = timeline ?: JSONObject()
            val video = timeline.optJSONObject("video")
            return ProjectView(
                name = settings.optString("name"),
                width = video?.optInt("width") ?: settings.optInt("width", 1080),
                height = video?.optInt("height") ?: settings.optInt("height", 1920),
                rateNum = video?.optLong("rateNum") ?: settings.optLong("rateNum", 30),
                rateDen = (video?.optLong("rateDen") ?: settings.optLong("rateDen", 1)).coerceAtLeast(1),
                canUndo = json.optBoolean("canUndo"),
                canRedo = json.optBoolean("canRedo"),
                media = (project.optJSONArray("media") ?: JSONArray()).objects().map {
                    MediaItem(
                        id = it.optString("id"),
                        path = it.optString("path"),
                        name = it.optString("name"),
                        kind = it.optString("kind"),
                        duration = if (it.isNull("duration")) null else it.optDouble("duration"),
                    )
                },
                tracks = (timeline.optJSONArray("tracks") ?: JSONArray()).objects().map {
                    Track(it.optString("id"), it.optBoolean("visible", true), it.optBoolean("muted"))
                },
                clips = (timeline.optJSONArray("clips") ?: JSONArray()).objects().map {
                    Clip(
                        id = it.optString("id"),
                        trackId = it.optString("trackId"),
                        mediaId = it.optString("mediaId"),
                        name = it.optString("name"),
                        kind = when (it.optString("kind")) {
                            "video" -> ClipKind.Video
                            "audio" -> ClipKind.Audio
                            "image" -> ClipKind.Image
                            "text" -> ClipKind.Text
                            "layer" -> ClipKind.Layer
                            else -> ClipKind.Other
                        },
                        start = it.optDouble("start", 0.0),
                        duration = it.optDouble("duration", 0.0),
                        sourceStart = it.optDouble("sourceStart", 0.0),
                        volume = it.optDouble("volume", 1.0),
                        speed = it.optDouble("speed", 1.0),
                        text = it.optJSONObject("text"),
                    )
                }.sortedBy { it.start },
                createdId = json.optString("createdId").ifEmpty { null },
            )
        }
    }
}

/** Something that happened to an export job: message.rs's `Event`. */
sealed interface ApiEvent {
    val job: String

    data class Progress(override val job: String, val frame: Long, val total: Long, val stage: String) : ApiEvent
    data class Analysing(override val job: String, val what: String, val fraction: Double) : ApiEvent
    data class Done(override val job: String, val output: String, val width: Int, val height: Int) : ApiEvent
    data class Failed(override val job: String, val code: String, val message: String) : ApiEvent

    companion object {
        fun parse(text: String): ApiEvent? {
            val json = try {
                JSONObject(text)
            } catch (_: Exception) {
                return null
            }
            val job = json.optString("job")
            return when (json.optString("event")) {
                "export.progress" -> Progress(job, json.optLong("frame"), json.optLong("total"), json.optString("stage"))
                "cutout.progress" -> Analysing(job, "cutout", json.optDouble("fraction"))
                "stabilize.progress" -> Analysing(job, "stabilize", json.optDouble("fraction"))
                "export.done" -> Done(job, json.optString("output"), json.optInt("width"), json.optInt("height"))
                "export.failed" -> {
                    val error = json.optJSONObject("error") ?: JSONObject()
                    Failed(job, error.optString("code", "failed"), error.optString("message"))
                }
                else -> null
            }
        }
    }
}

fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }
