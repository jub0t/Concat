// SPDX-License-Identifier: AGPL-3.0-or-later

package app.concat.editor.subtitle

import app.concat.editor.api.Clip
import app.concat.editor.api.ClipKind
import app.concat.editor.api.Commands
import app.concat.editor.api.ProjectView
import java.util.Locale
import kotlin.math.max
import kotlin.math.roundToInt
import org.json.JSONObject

data class SubtitleCue(
    val start: Double,
    val end: Double,
    val text: String,
    val style: JSONObject,
) {
    val duration: Double get() = max(0.05, end - start)
}

object SubtitleCodec {

    private data class AssStyle(
        val fontName: String = "Hanken Grotesk",
        val fontSize: Double = 48.0,
        val primaryColor: String = "#ffffff",
        val outlineColor: String = "#000000",
        val backColor: String? = null,
        val bold: Boolean = true,
        val italic: Boolean = false,
        val underline: Boolean = false,
        val strikeOut: Boolean = false,
        val borderStyle: Int = 1,
        val outline: Double = 2.0,
        val shadow: Double = 1.0,
        val alignment: Int = 2,
    )

    fun parse(raw: String, filename: String, frameHeight: Int): List<SubtitleCue> {
        val text = raw.removePrefix("\uFEFF").replace("\r\n", "\n").replace('\r', '\n')
        val isAss = filename.lowercase(Locale.ROOT).let { it.endsWith(".ass") || it.endsWith(".ssa") } ||
            text.contains("[Script Info]", ignoreCase = true) ||
            text.contains("[Events]", ignoreCase = true)
        return if (isAss) parseAss(text, frameHeight) else parseSrt(text)
    }

    // --- SRT PARSER ---

    private fun parseSrt(text: String): List<SubtitleCue> {
        val blocks = text.trim().split(Regex("\n\\s*\n"))
        val cues = mutableListOf<SubtitleCue>()
        val timeRegex = Regex("""(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})\s*-->\s*(\d{1,2}):(\d{2}):(\d{2})[,.](\d{1,3})""")

        for (block in blocks) {
            val lines = block.lines().map { it.trimEnd() }.filter { it.isNotEmpty() }
            if (lines.size < 2) continue
            val timeLineIdx = lines.indexOfFirst { timeRegex.containsMatchIn(it) }
            if (timeLineIdx == -1 || timeLineIdx + 1 >= lines.size) continue

            val match = timeRegex.find(lines[timeLineIdx]) ?: continue
            val start = parseTimestamp(match.groupValues[1], match.groupValues[2], match.groupValues[3], match.groupValues[4])
            val end = parseTimestamp(match.groupValues[5], match.groupValues[6], match.groupValues[7], match.groupValues[8])
            if (end <= start) continue

            val rawBody = lines.drop(timeLineIdx + 1).joinToString("\n")
            val italic = rawBody.contains("<i>", ignoreCase = true)
            val bold = rawBody.contains("<b>", ignoreCase = true)
            val underline = rawBody.contains("<u>", ignoreCase = true)
            val colorMatch = Regex("""<font[^>]*color=["']?(#[0-9a-fA-F]{6})["']?[^>]*>""", RegexOption.IGNORE_CASE).find(rawBody)
            val hexColor = colorMatch?.groupValues?.get(1)?.lowercase(Locale.ROOT) ?: "#ffffff"
            val cleanBody = rawBody.replace(Regex("<[^>]+>"), "").trim()
            if (cleanBody.isEmpty()) continue

            val style = buildTitleStyle(
                content = cleanBody,
                fontSize = 0.065,
                color = hexColor,
                fontWeight = if (bold) 700.0 else 500.0,
                italic = italic,
                underline = underline,
                strokeColor = "#000000",
                strokeWidth = 0.004,
                shadow = true,
            )
            cues += SubtitleCue(start, end, cleanBody, style)
        }
        return cues
    }

    // --- ASS / SSA PARSER ---

    private fun parseAss(text: String, frameHeight: Int): List<SubtitleCue> {
        var playResY = frameHeight.coerceAtLeast(360).toDouble()
        val styles = mutableMapOf("Default" to AssStyle())
        var section = ""
        var styleFormat = listOf<String>()
        var eventFormat = listOf<String>()
        val cues = mutableListOf<SubtitleCue>()

        for (rawLine in text.lines()) {
            val line = rawLine.trim()
            if (line.isEmpty() || line.startsWith(";")) continue
            if (line.startsWith("[") && line.endsWith("]")) {
                section = line.lowercase(Locale.ROOT)
                continue
            }
            when (section) {
                "[script info]" -> {
                    if (line.startsWith("PlayResY:", ignoreCase = true)) {
                        line.substringAfter(":").trim().toDoubleOrNull()?.takeIf { it > 0 }?.let { playResY = it }
                    }
                }
                "[v4+ styles]", "[v4 styles]" -> {
                    if (line.startsWith("Format:", ignoreCase = true)) {
                        styleFormat = line.substringAfter(":").split(",").map { it.trim().lowercase(Locale.ROOT) }
                    } else if (line.startsWith("Style:", ignoreCase = true) && styleFormat.isNotEmpty()) {
                        val parts = line.substringAfter(":").split(",").map { it.trim() }
                        fun col(name: String): String? = styleFormat.indexOf(name).takeIf { it in parts.indices }?.let { parts[it] }
                        val name = col("name") ?: "Default"
                        val borderStyle = col("borderstyle")?.toIntOrNull() ?: 1
                        val backHex = col("backcolour")?.let { assColorToHex(it) }
                        styles[name] = AssStyle(
                            fontName = col("fontname")?.ifBlank { "Hanken Grotesk" } ?: "Hanken Grotesk",
                            fontSize = col("fontsize")?.toDoubleOrNull() ?: 48.0,
                            primaryColor = col("primarycolour")?.let { assColorToHex(it) } ?: "#ffffff",
                            outlineColor = col("outlinecolour")?.let { assColorToHex(it) } ?: "#000000",
                            backColor = if (borderStyle == 3) backHex else null,
                            bold = (col("bold")?.toIntOrNull() ?: 0) != 0,
                            italic = (col("italic")?.toIntOrNull() ?: 0) != 0,
                            underline = (col("underline")?.toIntOrNull() ?: 0) != 0,
                            strikeOut = (col("strikeout")?.toIntOrNull() ?: 0) != 0,
                            borderStyle = borderStyle,
                            outline = col("outline")?.toDoubleOrNull() ?: 2.0,
                            shadow = col("shadow")?.toDoubleOrNull() ?: 0.0,
                            alignment = col("alignment")?.toIntOrNull() ?: 2,
                        )
                    }
                }
                "[events]" -> {
                    if (line.startsWith("Format:", ignoreCase = true)) {
                        eventFormat = line.substringAfter(":").split(",").map { it.trim().lowercase(Locale.ROOT) }
                    } else if (line.startsWith("Dialogue:", ignoreCase = true) && eventFormat.isNotEmpty()) {
                        val payload = line.substringAfter(":")
                        val parts = payload.split(",", limit = eventFormat.size).map { it.trim() }
                        if (parts.size < eventFormat.size) continue
                        fun col(name: String): String? = eventFormat.indexOf(name).takeIf { it in parts.indices }?.let { parts[it] }

                        val start = parseAssTime(col("start") ?: continue) ?: continue
                        val end = parseAssTime(col("end") ?: continue) ?: continue
                        if (end <= start) continue

                        val styleName = col("style") ?: "Default"
                        val base = styles[styleName] ?: styles["Default"] ?: AssStyle()
                        val rawDialogue = col("text") ?: continue

                        // Ekstrak override tag sederhana ({\c&H...&}, {\bord...}, {\shad...}, {\b1}, {\i1})
                        var color = base.primaryColor
                        var outlineColor = base.outlineColor
                        var outlinePx = base.outline
                        var shadowPx = base.shadow
                        var bold = base.bold
                        var italic = base.italic
                        var underline = base.underline
                        var fontScale = 1.0

                        Regex("""\{([^}]*)\}""").findAll(rawDialogue).forEach { block ->
                            val tags = block.groupValues[1]
                            Regex("""\\(?:1c|c)&H([0-9a-fA-F]{6,8})&?""").find(tags)?.let {
                                color = assColorToHex("&H${it.groupValues[1]}&")
                            }
                            Regex("""\\3c&H([0-9a-fA-F]{6,8})&?""").find(tags)?.let {
                                outlineColor = assColorToHex("&H${it.groupValues[1]}&")
                            }
                            Regex("""\\bord([0-9.]+)""").find(tags)?.groupValues?.get(1)?.toDoubleOrNull()?.let {
                                outlinePx = it
                            }
                            Regex("""\\shad([0-9.]+)""").find(tags)?.groupValues?.get(1)?.toDoubleOrNull()?.let {
                                shadowPx = it
                            }
                            Regex("""\\fs([0-9.]+)""").find(tags)?.groupValues?.get(1)?.toDoubleOrNull()?.let {
                                fontScale = it / base.fontSize.coerceAtLeast(1.0)
                            }
                            if (tags.contains("\\b1")) bold = true
                            if (tags.contains("\\b0")) bold = false
                            if (tags.contains("\\i1")) italic = true
                            if (tags.contains("\\i0")) italic = false
                            if (tags.contains("\\u1")) underline = true
                            if (tags.contains("\\u0")) underline = false
                        }

                        val cleanText = rawDialogue
                            .replace(Regex("""\{[^}]*\}"""), "")
                            .replace("\\N", "\n")
                            .replace("\\n", "\n")
                            .replace("\\h", " ")
                            .trim()
                        if (cleanText.isEmpty()) continue

                        val normFontSize = ((base.fontSize * fontScale) / playResY).coerceIn(0.02, 0.35)
                        val normStrokeWidth = (outlinePx / playResY).coerceIn(0.0, 0.05)
                        val align = when (base.alignment) {
                            1, 4, 7 -> "left"
                            3, 6, 9 -> "right"
                            else -> "center"
                        }

                        val titleJson = buildTitleStyle(
                            content = cleanText,
                            fontFamily = base.fontName,
                            fontSize = normFontSize,
                            color = color,
                            fontWeight = if (bold) 700.0 else 400.0,
                            italic = italic,
                            underline = underline,
                            strikethrough = base.strikeOut,
                            align = align,
                            strokeColor = outlineColor,
                            strokeWidth = normStrokeWidth,
                            shadow = shadowPx > 0.0,
                            plate = base.backColor,
                        )
                        cues += SubtitleCue(start, end, cleanText, titleJson)
                    }
                }
            }
        }
        return cues
    }

    /** Membangun batch command `addTextClip` dengan `above = true` agar overlap otomatis naik ke track baru tanpa di-flatten. */
    fun buildImportCommands(cues: List<SubtitleCue>): List<JSONObject> =
        cues.map { cue ->
            Commands.addTextClip(
                start = cue.start,
                style = cue.style,
                duration = cue.duration,
                trackId = null,
            )
        }

    // --- EXPORTER SRT & ASS ---

    fun exportSrt(view: ProjectView): String {
        val textClips = view.clips
            .filter { it.kind == ClipKind.Text }
            .sortedWith(compareBy<Clip> { it.start }.thenBy { it.trackId })
        val sb = StringBuilder()
        textClips.forEachIndexed { idx, clip ->
            val content = clip.text?.optString("content")?.ifBlank { clip.name } ?: clip.name
            sb.append(idx + 1).append("\r\n")
            sb.append(formatSrtTime(clip.start)).append(" --> ").append(formatSrtTime(clip.end)).append("\r\n")
            sb.append(content.replace("\n", "\r\n")).append("\r\n\r\n")
        }
        return sb.toString()
    }

    fun exportAss(view: ProjectView): String {
        val playResX = view.width.coerceAtLeast(640)
        val playResY = view.height.coerceAtLeast(360)
        val textClips = view.clips
            .filter { it.kind == ClipKind.Text }
            .sortedWith(compareBy<Clip> { it.start }.thenBy { it.trackId })

        val sb = StringBuilder()
        sb.appendLine("[Script Info]")
        sb.appendLine("Title: ${view.name.ifBlank { "Concat Export" }}")
        sb.appendLine("ScriptType: v4.00+")
        sb.appendLine("WrapStyle: 0")
        sb.appendLine("ScaledBorderAndShadow: yes")
        sb.appendLine("PlayResX: $playResX")
        sb.appendLine("PlayResY: $playResY")
        sb.appendLine()
        sb.appendLine("[V4+ Styles]")
        sb.appendLine("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding")

        val trackOrder = view.tracks.mapIndexed { i, t -> t.id to i }.toMap()
        val styleNames = mutableMapOf<String, String>()
        var styleCounter = 1

        for (clip in textClips) {
            val st = clip.text ?: JSONObject()
            val fontFamily = st.optString("fontFamily", "Hanken Grotesk").ifBlank { "Hanken Grotesk" }
            val fontSizePx = ((st.optDouble("fontSize", 0.07)) * playResY).roundToInt().coerceAtLeast(12)
            val colorHex = st.optString("color", "#ffffff")
            val strokeHex = st.optString("strokeColor", "#000000")
            val strokePx = ((st.optDouble("strokeWidth", 0.0)) * playResY).coerceAtLeast(0.0)
            val shadow = if (st.optBoolean("shadow", true)) 2.0 else 0.0
            val bold = if (st.optDouble("fontWeight", 700.0) >= 600.0) -1 else 0
            val italic = if (st.optBoolean("italic", false)) -1 else 0
            val underline = if (st.optBoolean("underline", false)) -1 else 0
            val strike = if (st.optBoolean("strikethrough", false)) -1 else 0
            val plate = st.optString("plate", "").takeIf { it.startsWith("#") }
            val borderStyle = if (plate != null) 3 else 1
            val backAss = plate?.let { hexToAssColor(it) } ?: "&H80000000"
            val align = when (st.optString("align", "center").lowercase(Locale.ROOT)) {
                "left" -> 1
                "right" -> 3
                else -> 2
            }

            val signature = "$fontFamily|$fontSizePx|$colorHex|$strokeHex|$strokePx|$shadow|$bold|$italic|$underline|$strike|$borderStyle|$backAss|$align"
            val styleName = styleNames.getOrPut(signature) {
                val name = "Style_${styleCounter++}"
                sb.appendLine(
                    String.format(
                        Locale.US,
                        "Style: %s,%s,%d,%s,&H000000FF,%s,%s,%d,%d,%d,%d,100,100,0,0,%d,%.2f,%.1f,%d,20,20,24,1",
                        name, fontFamily, fontSizePx, hexToAssColor(colorHex), hexToAssColor(strokeHex),
                        backAss, bold, italic, underline, strike, borderStyle, strokePx, shadow, align,
                    ),
                )
                name
            }

            val layer = trackOrder[clip.trackId] ?: 0
            val content = (st.optString("content").ifBlank { clip.name }).replace("\n", "\\N")
            val posX = ((0.5 + clip.offsetX) * playResX).roundToInt()
            val posY = ((0.5 + clip.offsetY) * playResY).roundToInt()
            val posTag = if (clip.offsetX != 0.0 || clip.offsetY != 0.0) "{\\pos($posX,$posY)}" else ""

            sb.appendLine(
                String.format(
                    Locale.US,
                    "Dialogue: %d,%s,%s,%s,,0,0,0,,%s%s",
                    layer,
                    formatAssTime(clip.start),
                    formatAssTime(clip.end),
                    styleName,
                    posTag,
                    content,
                ),
            )
        }
        return sb.toString()
    }

    // --- HELPERS ---

    fun buildTitleStyle(
        content: String,
        fontFamily: String = "Hanken Grotesk",
        fontSize: Double = 0.07,
        color: String = "#ffffff",
        fontWeight: Double = 700.0,
        italic: Boolean = false,
        underline: Boolean = false,
        strikethrough: Boolean = false,
        align: String = "center",
        strokeColor: String = "#000000",
        strokeWidth: Double = 0.0,
        shadow: Boolean = true,
        plate: String? = null,
    ): JSONObject = JSONObject()
        .put("content", content)
        .put("fontFamily", fontFamily)
        .put("fontSize", fontSize)
        .put("fontWeight", fontWeight)
        .put("italic", italic)
        .put("underline", underline)
        .put("strikethrough", strikethrough)
        .put("color", color)
        .put("opacity", 1.0)
        .put("align", align)
        .put("case", "original")
        .put("lineHeight", 1.2)
        .put("tracking", 0.0)
        .put("maxWidth", 0.0)
        .put("maxHeight", 0.0)
        .put("strokeColor", strokeColor)
        .put("strokeWidth", strokeWidth)
        .put("shadow", shadow)
        .put("plate", plate ?: JSONObject.NULL)
        .put("platePaddingX", 0.0315)
        .put("platePaddingY", 0.018)
        .put("plateRadius", 0.0135)

    private fun parseTimestamp(h: String, m: String, s: String, ms: String): Double {
        val millis = ms.padEnd(3, '0').take(3).toDoubleOrNull() ?: 0.0
        return (h.toDoubleOrNull() ?: 0.0) * 3600.0 +
            (m.toDoubleOrNull() ?: 0.0) * 60.0 +
            (s.toDoubleOrNull() ?: 0.0) +
            millis / 1000.0
    }

    private fun parseAssTime(raw: String): Double? {
        val parts = raw.trim().split(":")
        if (parts.size != 3) return null
        val h = parts[0].toDoubleOrNull() ?: return null
        val m = parts[1].toDoubleOrNull() ?: return null
        val s = parts[2].replace(',', '.').toDoubleOrNull() ?: return null
        return h * 3600.0 + m * 60.0 + s
    }

    private fun formatSrtTime(seconds: Double): String {
        val totalMs = max(0L, (seconds * 1000.0).roundToInt().toLong())
        val h = totalMs / 3_600_000
        val m = (totalMs % 3_600_000) / 60_000
        val s = (totalMs % 60_000) / 1000
        val ms = totalMs % 1000
        return String.format(Locale.US, "%02d:%02d:%02d,%03d", h, m, s, ms)
    }

    private fun formatAssTime(seconds: Double): String {
        val totalCs = max(0L, (seconds * 100.0).roundToInt().toLong())
        val h = totalCs / 360_000
        val m = (totalCs % 360_000) / 6000
        val s = (totalCs % 6000) / 100
        val cs = totalCs % 100
        return String.format(Locale.US, "%d:%02d:%02d.%02d", h, m, s, cs)
    }

    private fun assColorToHex(ass: String): String {
        val clean = ass.trim().removePrefix("&H").removePrefix("&h").removeSuffix("&")
        val padded = clean.padStart(6, '0')
        val bgr = if (padded.length >= 8) padded.substring(2, 8) else padded.takeLast(6)
        val bb = bgr.substring(0, 2)
        val gg = bgr.substring(2, 4)
        val rr = bgr.substring(4, 6)
        return "#$rr$gg$bb".lowercase(Locale.ROOT)
    }

    private fun hexToAssColor(hex: String): String {
        val clean = hex.trim().removePrefix("#")
        if (clean.length < 6) return "&H00FFFFFF"
        val rr = clean.substring(0, 2).uppercase(Locale.ROOT)
        val gg = clean.substring(2, 4).uppercase(Locale.ROOT)
        val bb = clean.substring(4, 6).uppercase(Locale.ROOT)
        return "&H00$bb$gg$rr"
    }
}
