// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import app.concat.editor.spike.api.Clip
import app.concat.editor.spike.ui.components.ChoiceChip
import app.concat.editor.spike.ui.components.PanelLabel
import app.concat.editor.spike.ui.components.ToolPanel
import app.concat.editor.spike.ui.components.ValueSlider
import app.concat.editor.spike.ui.theme.Spacing
import org.json.JSONObject

/*
 * The clip's look beyond its placement: crop, mask and animation. Each is a
 * field of `updateClip`'s patch (`crop`, `masks`, `motion`), so every
 * change here is one undo step, sent when a slider is let go or a chip is
 * tapped. JSON `null` takes a crop or the animation off.
 */

/** A row of chips that scrolls sideways when it is wider than the panel. */
@Composable
private fun <T> Chips(options: List<T>, selected: T, label: (T) -> String, onSelect: (T) -> Unit) {
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(Spacing.xs),
    ) {
        for (option in options) ChoiceChip(label(option), option == selected, onClick = { onSelect(option) })
    }
}

private fun JSONObject?.number(name: String, default: Double): Double =
    this?.optDouble(name, default)?.takeIf { !it.isNaN() } ?: default

/**
 * How much is cut off each side, as fractions of the picture. Each side
 * stops short of meeting its opposite, so something is always left.
 */
@Composable
fun CropPanel(clip: Clip, onApply: (JSONObject) -> Unit, onClose: () -> Unit) {
    val crop = clip.crop
    var left by remember(clip.id) { mutableFloatStateOf(crop.number("left", 0.0).toFloat()) }
    var top by remember(clip.id) { mutableFloatStateOf(crop.number("top", 0.0).toFloat()) }
    var right by remember(clip.id) { mutableFloatStateOf(crop.number("right", 0.0).toFloat()) }
    var bottom by remember(clip.id) { mutableFloatStateOf(crop.number("bottom", 0.0).toFloat()) }
    fun send() {
        val none = left <= 0f && top <= 0f && right <= 0f && bottom <= 0f
        onApply(
            JSONObject().put(
                "crop",
                if (none) JSONObject.NULL
                else JSONObject().put("left", left.toDouble()).put("top", top.toDouble())
                    .put("right", right.toDouble()).put("bottom", bottom.toDouble()),
            ),
        )
    }
    fun percent(value: Float) = "${(value * 100).toInt()}%"
    // Each side may take what its opposite leaves, less a tenth of the picture.
    fun room(opposite: Float) = 0f..(0.9f - opposite).coerceAtLeast(0f)
    ToolPanel(
        "Crop",
        onClose = onClose,
        onConfirm = onClose,
        actions = {
            ChoiceChip("Reset", false, onClick = {
                left = 0f; top = 0f; right = 0f; bottom = 0f
                send()
            })
        },
    ) {
        ValueSlider("Left", left, room(right), ::percent, onChange = { left = it }, onDone = ::send)
        ValueSlider("Right", right, room(left), ::percent, onChange = { right = it }, onDone = ::send)
        ValueSlider("Top", top, room(bottom), ::percent, onChange = { top = it }, onDone = ::send)
        ValueSlider("Bottom", bottom, room(top), ::percent, onChange = { bottom = it }, onDone = ::send)
    }
}

/** The shapes a mask is drawn from, by the document's names; null is no mask. */
private val MASK_SHAPES = listOf(null, "linear", "mirror", "circle", "rectangle", "heart", "star")

private fun shapeLabel(shape: String?) = when (shape) {
    null -> "None"
    "linear" -> "Line"
    "mirror" -> "Band"
    "rectangle" -> "Box"
    else -> shape.replaceFirstChar { it.uppercase() }
}

/**
 * One mask, CapCut's way: a shape, how soft its edge is, and whether it
 * keeps the inside or the outside. A clip that already has several keeps
 * the rest; this edits the top one. Its place and size are kept as they
 * were, so a mask moved elsewhere (on the desktop) stays where it is.
 */
@Composable
fun MaskPanel(clip: Clip, onApply: (JSONObject) -> Unit, onClose: () -> Unit) {
    val first = clip.masks.firstOrNull()
    var shape by remember(clip.id) { mutableStateOf(first?.optJSONObject("source")?.optString("shape")?.ifEmpty { null }) }
    var feather by remember(clip.id) { mutableFloatStateOf(first.number("feather", 0.0).toFloat()) }
    var invert by remember(clip.id) { mutableStateOf(first?.optBoolean("invert") ?: false) }
    fun send() {
        val rest = clip.masks.drop(1)
        val masks = org.json.JSONArray()
        val chosen = shape
        if (chosen != null) {
            // A new shape is a new mask (its own size); the same shape keeps its numbers.
            val sameShape = first?.optJSONObject("source")?.optString("shape") == chosen
            val mask = if (first != null && sameShape) JSONObject(first.toString())
            else JSONObject().apply { first?.optString("id")?.takeIf { it.isNotEmpty() }?.let { put("id", it) } }
                .put("source", JSONObject().put("kind", "shape").put("shape", chosen))
            mask.put("feather", feather.toDouble()).put("invert", invert)
            masks.put(mask)
        }
        rest.forEach { masks.put(it) }
        onApply(JSONObject().put("masks", masks))
    }
    ToolPanel("Mask", onClose = onClose, onConfirm = onClose) {
        Chips(MASK_SHAPES, shape, ::shapeLabel) {
            shape = it
            send()
        }
        if (shape != null) {
            ValueSlider("Feather", feather, 0f..1f, { "${(it * 100).toInt()}%" }, onChange = { feather = it }, onDone = ::send)
            Row(horizontalArrangement = Arrangement.spacedBy(Spacing.xs)) {
                ChoiceChip("Keep inside", !invert, onClick = { invert = false; send() })
                ChoiceChip("Keep outside", invert, onClick = { invert = true; send() })
            }
        }
    }
}

/** Which of a clip's three motions the Animation panel is showing. */
private enum class Phase(val field: String, val title: String) { Enter("in", "In"), Exit("out", "Out"), Loop("loop", "Loop") }

private val ENTRANCES = listOf("fade", "slide-left", "slide-right", "slide-up", "slide-down", "zoom-in", "zoom-out", "pop", "drop", "rise", "spin", "flip", "swing", "squash")
private val LOOPS = listOf("pulse", "heartbeat", "float", "wiggle", "sway", "jitter", "spin", "flicker", "breathe")

private fun moveLabel(kind: String?) = kind?.replace('-', ' ')?.replaceFirstChar { it.uppercase() } ?: "None"

/**
 * How the clip comes on, goes off, and moves all along (`motion`): a move
 * by name for each, and how long it takes. All three empty takes the
 * animation off.
 */
@Composable
fun AnimationPanel(clip: Clip, onApply: (JSONObject) -> Unit, onClose: () -> Unit) {
    var phase by remember(clip.id) { mutableStateOf(Phase.Enter) }
    // Per phase: the move's name (null for none) and its seconds.
    val motion = clip.motion
    val kinds = remember(clip.id) {
        Phase.entries.associateWith { motion?.optJSONObject(it.field)?.optString("kind")?.ifEmpty { null } }.toMutableMap()
    }
    val seconds = remember(clip.id) {
        Phase.entries.associateWith { (motion?.optJSONObject(it.field)).number("seconds", if (it == Phase.Loop) 1.0 else 0.5).toFloat() }.toMutableMap()
    }
    var shown by remember(clip.id) { mutableStateOf(kinds[Phase.Enter]) }
    var length by remember(clip.id) { mutableFloatStateOf(seconds.getValue(Phase.Enter)) }
    fun send() {
        kinds[phase] = shown
        seconds[phase] = length
        val out = JSONObject()
        for (each in Phase.entries) {
            val kind = kinds[each] ?: continue
            val move = JSONObject().put("kind", kind).put("seconds", seconds.getValue(each).toDouble())
            if (each == Phase.Loop) move.put("amount", motion?.optJSONObject("loop").number("amount", 1.0))
            out.put(each.field, move)
        }
        onApply(JSONObject().put("motion", if (out.length() == 0) JSONObject.NULL else out))
    }
    // An entrance and an exit share the clip; each may take up to half of it.
    val most = if (phase == Phase.Loop) 4f else (clip.duration / 2).coerceIn(0.1, 3.0).toFloat()
    ToolPanel("Animation", onClose = onClose, onConfirm = onClose) {
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.xs)) {
            for (each in Phase.entries) {
                ChoiceChip(each.title + if (kinds[each] != null) " •" else "", phase == each, onClick = {
                    phase = each
                    shown = kinds[each]
                    length = seconds.getValue(each)
                })
            }
        }
        PanelLabel(if (phase == Phase.Loop) "All along" else if (phase == Phase.Enter) "As it comes on" else "As it goes off")
        Chips(listOf<String?>(null) + if (phase == Phase.Loop) LOOPS else ENTRANCES, shown, ::moveLabel) {
            shown = it
            send()
        }
        if (shown != null) {
            ValueSlider(
                if (phase == Phase.Loop) "One lap" else "Length",
                length.coerceAtMost(most),
                0.1f..most,
                { "%.1fs".format(it) },
                onChange = { length = it },
                onDone = ::send,
            )
        }
    }
}
