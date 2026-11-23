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

enum class ClipKind {
    Video, Audio, Image, Text, Layer, SoundLayer, Transition, Other;

    /** Whether the clip plays sound of its own: what captions can hear. */
    val hasSound get() = this == Video || this == Audio
}

/**
 * One link of a clip's effect chain (`AppliedFilter`): the package, its
 * knobs as set, and the whole object, so a chain sent back keeps what the
 * phone does not edit (keys).
 */
data class AppliedEffect(val id: String, val params: Map<String, Double>, val enabled: Boolean, val raw: JSONObject) {
    /** The same link with one knob set. */
    fun withParam(key: String, value: Double): JSONObject {
        val out = JSONObject(raw.toString())
        val params = out.optJSONObject("params") ?: JSONObject()
        out.put("params", params.put(key, value))
        return out
    }

    companion object {
        fun from(json: JSONObject): AppliedEffect {
            val params = json.optJSONObject("params")
            val map = LinkedHashMap<String, Double>()
            params?.keys()?.forEach { key -> map[key] = params.optDouble(key) }
            return AppliedEffect(json.optString("id"), map, json.optBoolean("enabled", true), json)
        }

        /** A new link at the package's defaults. */
        fun fresh(id: String): JSONObject = JSONObject().put("id", id).put("params", JSONObject()).put("enabled", true)

        fun list(array: JSONArray?): List<AppliedEffect> = array?.objects()?.map(::from) ?: emptyList()
    }
}

data class MediaItem(
    val id: String,
    val path: String,
    val name: String,
    val kind: String,
    val duration: Double?,
    /** The picture's size, when the probe found one. */
    val width: Int = 0,
    val height: Int = 0,
)

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
    /** The picture's chain, in order; a transition clip's package is its first link. */
    val videoEffects: List<AppliedEffect> = emptyList(),
    /** The sound's chain, in order. */
    val filters: List<AppliedEffect> = emptyList(),
    /** Placement (concat-project's `Clip`): a multiplier over the fitted size. */
    val scale: Double = 1.0,
    /** Offset from centred, as a fraction of the frame's width; positive is right. */
    val offsetX: Double = 0.0,
    /** The same down the frame's height; positive is down. */
    val offsetY: Double = 0.0,
    /** Clockwise degrees about the picture's centre. */
    val rotation: Double = 0.0,
    val opacity: Double = 1.0,
    val muted: Boolean = false,
    val flipH: Boolean = false,
    val fadeIn: Double = 0.0,
    val fadeOut: Double = 0.0,
    /** The instants keyed, `0..=1` into the clip, of each property that has keys. */
    val keys: Map<String, List<Double>> = emptyMap(),
) {
    val end get() = start + duration

    /** Whether the preview can move it about: a title, a picture, a layer over the main track. */
    val placeable get() = kind == ClipKind.Text || kind == ClipKind.Image || kind == ClipKind.Video || kind == ClipKind.Other

    /** Whether any property is keyed within half a frame of [at] (0..1 into the clip). */
    fun keyedAt(at: Double, tolerance: Double): Boolean = keys.values.any { list -> list.any { kotlin.math.abs(it - at) <= tolerance } }
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
    /**
     * The active timeline's lanes as the document stores them: bottom
     * first, the last one drawn on top (concat-project's `AddTrack`).
     */
    val tracks: List<Track>,
    val clips: List<Clip>,
    val createdId: String?,
    /** The active timeline's id: what the timeline commands name. */
    val timelineId: String = "",
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
                        width = it.optInt("width"),
                        height = it.optInt("height"),
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
                            "soundLayer" -> ClipKind.SoundLayer
                            "transition" -> ClipKind.Transition
                            else -> ClipKind.Other
                        },
                        start = it.optDouble("start", 0.0),
                        duration = it.optDouble("duration", 0.0),
                        sourceStart = it.optDouble("sourceStart", 0.0),
                        volume = it.optDouble("volume", 1.0),
                        speed = it.optDouble("speed", 1.0),
                        text = it.optJSONObject("text"),
                        videoEffects = AppliedEffect.list(it.optJSONArray("videoEffects")),
                        filters = AppliedEffect.list(it.optJSONArray("filters")),
                        scale = it.optDouble("scale", 1.0).takeIf { v -> !v.isNaN() } ?: 1.0,
                        offsetX = it.optDouble("offsetX", 0.0).takeIf { v -> !v.isNaN() } ?: 0.0,
                        offsetY = it.optDouble("offsetY", 0.0).takeIf { v -> !v.isNaN() } ?: 0.0,
                        rotation = it.optDouble("rotation", 0.0).takeIf { v -> !v.isNaN() } ?: 0.0,
                        opacity = it.optDouble("opacity", 1.0).takeIf { v -> !v.isNaN() } ?: 1.0,
                        muted = it.optBoolean("muted"),
                        flipH = it.optBoolean("flipH"),
                        fadeIn = it.optDouble("fadeIn", 0.0).takeIf { v -> !v.isNaN() } ?: 0.0,
                        fadeOut = it.optDouble("fadeOut", 0.0).takeIf { v -> !v.isNaN() } ?: 0.0,
                        keys = keysOf(it.optJSONArray("keys")),
                    )
                }.sortedBy { it.start },
                createdId = json.optString("createdId").ifEmpty { null },
                timelineId = timeline.optString("id"),
            )
        }

        /** `[{"property": "scale", "at": 0.2, "value": 1.1, "ease": ...}]`: the instants each property is keyed at. */
        private fun keysOf(json: JSONArray?): Map<String, List<Double>> {
            if (json == null) return emptyMap()
            return json.objects().filter { !it.optDouble("at").isNaN() }
                .groupBy({ it.optString("property") }, { it.optDouble("at") })
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

/** One speech model: `models.list`'s `ModelInfo`. */
data class ModelInfo(
    val id: String,
    val label: String,
    val blurb: String,
    val family: String,
    val englishOnly: Boolean,
    val sizeBytes: Long,
    val downloaded: Boolean,
) {
    companion object {
        fun from(json: JSONObject) = ModelInfo(
            id = json.optString("id"),
            label = json.optString("label").ifBlank { json.optString("id") },
            blurb = json.optString("blurb"),
            family = json.optString("family"),
            englishOnly = json.optBoolean("englishOnly"),
            sizeBytes = json.optLong("sizeBytes"),
            downloaded = json.optBoolean("downloaded"),
        )
    }
}

data class VoiceItem(val id: Int, val name: String, val family: String)

/** `models.list`: what hears (captions), what reads aloud, and the voices. */
data class ModelsInfo(val transcribers: List<ModelInfo>, val speech: List<ModelInfo>, val voices: List<VoiceItem>) {
    companion object {
        val EMPTY = ModelsInfo(emptyList(), emptyList(), emptyList())

        fun from(json: JSONObject) = ModelsInfo(
            transcribers = (json.optJSONArray("transcribers") ?: JSONArray()).objects().map(ModelInfo::from),
            speech = (json.optJSONArray("speech") ?: JSONArray()).objects().map(ModelInfo::from),
            voices = (json.optJSONArray("voices") ?: JSONArray()).objects().map {
                VoiceItem(it.optInt("id"), it.optString("name"), it.optString("family"))
            },
        )
    }
}

/** One knob of a package: `ParamInfo`. */
data class ParamInfo(
    val key: String,
    val label: String,
    val type: String,
    val min: Double,
    val max: Double,
    val default: Double,
    val step: Double,
    val unit: String,
    val values: List<Double>,
    val labels: List<String>,
) {
    /** What a slider can show: numbers, a switch, a choice. Colours and points are left to the desktop. */
    val sliderable get() = type == "float" || type == "int" || type == "bool" || type == "enum"

    companion object {
        fun from(json: JSONObject): ParamInfo {
            val values = json.optJSONArray("values")
            val labels = json.optJSONArray("labels")
            return ParamInfo(
                key = json.optString("key"),
                label = json.optString("label").ifBlank { json.optString("key") },
                type = json.optString("type", "float"),
                min = json.optDouble("min", 0.0),
                max = json.optDouble("max", 1.0),
                default = json.optDouble("default", 0.0),
                step = json.optDouble("step", 0.0),
                unit = json.optString("unit"),
                values = values?.let { a -> (0 until a.length()).map { a.optDouble(it) } } ?: emptyList(),
                labels = labels?.let { a -> (0 until a.length()).map { a.optString(it) } } ?: emptyList(),
            )
        }
    }
}

/** One effect, filter, sound effect or transition: `catalogue.list`'s `PackageInfo`. */
data class PackageInfo(
    val id: String,
    val name: String,
    val kind: String,
    val category: String,
    val description: String,
    val intensity: String?,
    val params: List<ParamInfo>,
) {
    companion object {
        fun from(json: JSONObject) = PackageInfo(
            id = json.optString("id"),
            name = json.optString("name").ifBlank { json.optString("id") },
            kind = json.optString("kind"),
            category = json.optString("category").ifBlank { "Other" },
            description = json.optString("description"),
            intensity = if (json.isNull("intensity")) null else json.optString("intensity").ifEmpty { null },
            params = (json.optJSONArray("params") ?: JSONArray()).objects().map(ParamInfo::from),
        )
    }
}

/** One stretch of speech heard in a clip, seconds into the clip's sound. */
data class CaptionSegment(val start: Double, val end: Double, val text: String) {
    fun toJson(): JSONObject = JSONObject().put("start", start).put("end", end).put("text", text)

    companion object {
        fun list(array: JSONArray?): List<CaptionSegment> = array?.objects()?.map {
            CaptionSegment(it.optDouble("start"), it.optDouble("end"), it.optString("text"))
        } ?: emptyList()
    }
}

/** `version`: the build and the contract it speaks. */
data class VersionInfo(val apiVersion: String, val concat: String, val capabilities: List<String>)

fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }
