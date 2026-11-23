// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.animation.core.FloatExponentialDecaySpec
import androidx.compose.animation.core.animateDecay
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.concat.editor.spike.api.Clip
import app.concat.editor.spike.api.ClipKind
import app.concat.editor.spike.api.ProjectView
import app.concat.editor.spike.ui.Palette
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Where a dragged clip lands: a lane that exists, or a new one above or below them all. */
sealed interface LaneTarget {
    data class Existing(val trackId: String) : LaneTarget
    data object NewTop : LaneTarget
    data object NewBottom : LaneTarget
}

/** What a finger is doing to a clip right now, drawn over the edit until it lifts. */
private data class DragPreview(
    val clipId: String,
    val start: Double,
    val duration: Double,
    /** Display row: -1 above the lanes (a new top lane), `rows` below them. */
    val row: Int,
    val snappedAt: Double?,
)

/**
 * Rows on screen, top first, over the document's lanes, which are stored
 * bottom first: row 0 is the last track.
 */
private class Lanes(val count: Int, val top: Float, val height: Float) {
    fun rowOf(y: Float): Int = floor((y - top) / height).toInt()
    fun rowTop(row: Int) = top + row * height
    fun trackIndex(row: Int) = count - 1 - row
    fun rowOfTrack(index: Int) = count - 1 - index
}

private const val MIN_LENGTH = 1.0 / 60

/**
 * The timeline: the playhead fixed at the centre and the edit sliding under
 * it. A drag scrubs and flings, a pinch zooms, a tap selects the clip under
 * the finger (or nothing). A long press on a clip picks it up to move it,
 * onto another lane or past the top or bottom one for a new lane; the
 * selected clip's handles trim its ends. Moves and trims snap to the
 * playhead and to the other clips' edges.
 */
@Composable
fun Timeline(
    view: ProjectView,
    position: Double,
    selected: String?,
    onScrub: (Double) -> Unit,
    onSelect: (String?) -> Unit,
    onMove: (clipId: String, start: Double, target: LaneTarget) -> Unit,
    onTrim: (clipId: String, edge: String, delta: Double) -> Unit,
    modifier: Modifier = Modifier,
) {
    var pxPerSecond by remember { mutableFloatStateOf(0f) }
    var drag by remember { mutableStateOf<DragPreview?>(null) }
    var fling by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val measurer = rememberTextMeasurer()
    val currentView by rememberUpdatedState(view)
    val currentPosition by rememberUpdatedState(position)
    val currentSelected by rememberUpdatedState(selected)
    val scrub by rememberUpdatedState(onScrub)
    val choose by rememberUpdatedState(onSelect)
    val move by rememberUpdatedState(onMove)
    val trim by rememberUpdatedState(onTrim)
    val ruler = 22.dp
    val laneMax = 52.dp

    Canvas(
        modifier
            .fillMaxSize()
            .pointerInput(Unit) {
                fun scaleNow() = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
                fun lanes() = max(1, currentView.tracks.size).let { count ->
                    Lanes(count, ruler.toPx(), min(laneMax.toPx(), (size.height - ruler.toPx()) / count))
                }
                val snapPx = 12.dp.toPx()
                val handlePx = 22.dp.toPx()

                /** The nearest snap point to any of [times] within reach, as (point, which time). */
                fun snap(times: List<Double>, exclude: String, scale: Float, playhead: Double): Pair<Double, Int>? {
                    val points = ArrayList<Double>()
                    points += playhead
                    points += 0.0
                    for (clip in currentView.clips) {
                        if (clip.id == exclude) continue
                        points += clip.start
                        points += clip.end
                    }
                    var best: Pair<Double, Int>? = null
                    var bestDistance = snapPx / scale.toDouble()
                    times.forEachIndexed { which, time ->
                        for (point in points) {
                            val distance = abs(point - time)
                            if (distance < bestDistance) {
                                bestDistance = distance
                                best = point to which
                            }
                        }
                    }
                    return best
                }

                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    fling?.cancel()
                    val view = currentView
                    val scale = scaleNow()
                    val playhead = currentPosition
                    val lanes = lanes()
                    val centre = size.width / 2f
                    fun timeAt(x: Float) = playhead + (x - centre) / scale
                    val downRow = lanes.rowOf(down.position.y)
                    val downTrack = view.tracks.getOrNull(lanes.trackIndex(downRow))
                    val downTime = timeAt(down.position.x)
                    val hit = view.clips.firstOrNull { it.trackId == downTrack?.id && downTime >= it.start && downTime < it.end }

                    // A handle of the selected clip: trim that end.
                    val chosen = view.clip(currentSelected)
                    val edge = chosen?.let { clip ->
                        val row = lanes.rowOfTrack(view.tracks.indexOfFirst { it.id == clip.trackId })
                        val inLane = downRow == row
                        val left = centre + ((clip.start - playhead) * scale).toFloat()
                        val right = centre + ((clip.end - playhead) * scale).toFloat()
                        when {
                            !inLane -> null
                            abs(down.position.x - left) < handlePx && down.position.x < (left + right) / 2 -> "start"
                            abs(down.position.x - right) < handlePx -> "end"
                            else -> null
                        }
                    }
                    if (chosen != null && edge != null) {
                        down.consume()
                        val row = lanes.rowOfTrack(view.tracks.indexOfFirst { it.id == chosen.trackId })
                        val (low, high) = trimRange(view, chosen, edge)
                        var delta = 0.0
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) break
                            change.consume()
                            val raw = ((change.position.x - down.position.x) / scale).toDouble().coerceIn(low, high)
                            val moving = if (edge == "start") chosen.start + raw else chosen.end + raw
                            val snapped = snap(listOf(moving), chosen.id, scale, playhead)
                            delta = snapped?.let { (point, _) -> (point - if (edge == "start") chosen.start else chosen.end).coerceIn(low, high) } ?: raw
                            drag = if (edge == "start") {
                                DragPreview(chosen.id, chosen.start + delta, chosen.duration - delta, row, snapped?.first)
                            } else {
                                DragPreview(chosen.id, chosen.start, chosen.duration + delta, row, snapped?.first)
                            }
                        }
                        drag = null
                        if (abs(delta) > 1e-4) trim(chosen.id, edge, delta)
                        return@awaitEachGesture
                    }

                    // Wait to see what this touch is: a tap, a drag, or a press held still.
                    val slop = viewConfiguration.touchSlop
                    var outcome = Outcome.LongPress
                    withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                        while (true) {
                            val event = awaitPointerEvent()
                            if (event.changes.count { it.pressed } > 1) {
                                outcome = Outcome.Pan
                                break
                            }
                            val change = event.changes.firstOrNull { it.id == down.id }
                            if (change == null || !change.pressed) {
                                outcome = Outcome.Tap
                                break
                            }
                            if ((change.position - down.position).getDistance() > slop) {
                                outcome = Outcome.Pan
                                break
                            }
                        }
                    }

                    if (outcome == Outcome.Tap) {
                        choose(hit?.id)
                        return@awaitEachGesture
                    }

                    if (outcome == Outcome.LongPress && hit != null) {
                        // Picked up: follow the finger, then drop.
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        choose(hit.id)
                        val grab = downTime - hit.start
                        var start = hit.start
                        var row = lanes.rowOfTrack(view.tracks.indexOfFirst { it.id == hit.trackId })
                        drag = DragPreview(hit.id, start, hit.duration, row, null)
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) break
                            change.consume()
                            val wanted = max(0.0, timeAt(change.position.x) - grab)
                            val snapped = snap(listOf(wanted, wanted + hit.duration), hit.id, scale, playhead)
                            start = when {
                                snapped == null -> wanted
                                snapped.second == 0 -> snapped.first
                                else -> snapped.first - hit.duration
                            }.coerceAtLeast(0.0)
                            row = lanes.rowOf(change.position.y).coerceIn(-1, lanes.count)
                            drag = DragPreview(hit.id, start, hit.duration, row, snapped?.first)
                        }
                        drag = null
                        val target = when {
                            row < 0 -> LaneTarget.NewTop
                            row >= lanes.count -> LaneTarget.NewBottom
                            else -> LaneTarget.Existing(view.tracks[lanes.trackIndex(row)].id)
                        }
                        val sameLane = target is LaneTarget.Existing && target.trackId == hit.trackId
                        if (!sameLane || abs(start - hit.start) > 1e-4) move(hit.id, start, target)
                        return@awaitEachGesture
                    }

                    // Scrub and zoom, then fling what is left of a one-finger drag.
                    val tracker = VelocityTracker()
                    tracker.addPosition(down.uptimeMillis, down.position)
                    var at = playhead
                    var pinched = false
                    while (true) {
                        val event = awaitPointerEvent()
                        if (event.changes.none { it.pressed }) break
                        if (event.changes.count { it.pressed } > 1) pinched = true
                        val zoom = event.calculateZoom()
                        if (zoom != 1f) pxPerSecond = (scaleNow() * zoom).coerceIn(8.dp.toPx(), 600.dp.toPx())
                        val pan = event.calculatePan()
                        if (pan.x != 0f) {
                            at = (at - pan.x / scaleNow()).coerceIn(0.0, currentView.duration)
                            scrub(at)
                        }
                        event.changes.firstOrNull { it.id == down.id }?.let { tracker.addPosition(it.uptimeMillis, it.position) }
                        event.changes.forEach { it.consume() }
                    }
                    if (!pinched) {
                        val velocity = tracker.calculateVelocity().x
                        if (abs(velocity) > 200f) {
                            val from = at
                            val flingScale = scaleNow()
                            val end = currentView.duration
                            fling = scope.launch {
                                animateDecay(0f, -velocity, FloatExponentialDecaySpec()) { value, _ ->
                                    scrub((from + value / flingScale).coerceIn(0.0, end))
                                }
                            }
                        }
                    }
                }
            },
    ) {
        val scale = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
        val centre = size.width / 2f
        val top = ruler.toPx()
        val count = max(1, view.tracks.size)
        val lanes = Lanes(count, top, min(laneMax.toPx(), (size.height - top) / count))
        fun x(seconds: Double) = centre + ((seconds - position) * scale).toFloat()

        drawRect(Palette.background)
        drawRuler(measurer, position, scale, centre, top)

        val moving = drag
        view.tracks.forEachIndexed { index, track ->
            val laneTop = lanes.rowTop(lanes.rowOfTrack(index))
            drawRect(Palette.surface, Offset(0f, laneTop + 2f), Size(size.width, lanes.height - 4f))
            for (clip in view.clips) {
                if (clip.trackId != track.id) continue
                val beingMoved = moving?.clipId == clip.id
                drawClip(
                    measurer, clip, x(clip.start), x(clip.end), laneTop, lanes.height,
                    alpha = if (!track.visible) 0.4f else if (beingMoved) 0.3f else 1f,
                    selected = clip.id == selected && !beingMoved,
                )
            }
        }
        if (moving != null) {
            val clip = view.clip(moving.clipId)
            if (clip != null) {
                val row = moving.row
                val laneTop = lanes.rowTop(row.coerceIn(-1, count))
                if (row < 0 || row >= count) {
                    // A new lane is made where it drops.
                    val y = if (row < 0) top else lanes.rowTop(count)
                    drawLine(Palette.accent, Offset(0f, y), Offset(size.width, y), strokeWidth = 2.dp.toPx(),
                        pathEffect = PathEffect.dashPathEffect(floatArrayOf(12f, 8f)))
                }
                drawClip(
                    measurer, clip, x(moving.start), x(moving.start + moving.duration),
                    if (row < 0) top - lanes.height / 2f else if (row >= count) lanes.rowTop(count) else laneTop,
                    lanes.height, alpha = 0.9f, selected = true,
                )
            }
            moving.snappedAt?.let { at ->
                drawLine(Palette.accent, Offset(x(at), top), Offset(x(at), size.height), strokeWidth = 1.5.dp.toPx())
            }
        }
        if (view.clips.isEmpty()) {
            drawText(
                measurer,
                "Add media to start",
                Offset(centre + 12.dp.toPx(), top + lanes.height / 2f - 8.dp.toPx()),
                TextStyle(color = Palette.muted, fontSize = 13.sp),
            )
        }

        // The playhead.
        drawLine(Color.White, Offset(centre, 0f), Offset(centre, size.height), strokeWidth = 2.dp.toPx())
        drawCircle(Color.White, 4.dp.toPx(), Offset(centre, 4.dp.toPx()))
    }
}

private enum class Outcome { Tap, Pan, LongPress }

/**
 * How far an edge may go, in signed seconds: never shorter than a frame
 * at 60, never onto the neighbour on the lane, and for footage never past
 * what the file holds.
 */
private fun trimRange(view: ProjectView, clip: Clip, edge: String): Pair<Double, Double> {
    val lane = view.clips.filter { it.trackId == clip.trackId && it.id != clip.id }
    val footage = clip.kind == ClipKind.Video || clip.kind == ClipKind.Audio
    val mediaLength = view.mediaOf(clip)?.duration
    return if (edge == "start") {
        val previousEnd = lane.filter { it.end <= clip.start + 1e-6 }.maxOfOrNull { it.end } ?: 0.0
        var low = previousEnd - clip.start
        if (footage) low = max(low, -clip.sourceStart / clip.speed.coerceAtLeast(1e-6))
        low to (clip.duration - MIN_LENGTH)
    } else {
        val nextStart = lane.filter { it.start >= clip.end - 1e-6 }.minOfOrNull { it.start }
        var high = (nextStart ?: Double.MAX_VALUE) - clip.end
        if (footage && mediaLength != null) {
            high = min(high, (mediaLength - clip.sourceStart) / clip.speed.coerceAtLeast(1e-6) - clip.duration)
        }
        -(clip.duration - MIN_LENGTH) to max(0.0, high)
    }
}

/** A tick a second (or wider apart when zoomed out), labelled. */
private fun DrawScope.drawRuler(measurer: TextMeasurer, position: Double, scale: Float, centre: Float, top: Float) {
    val step = when {
        scale > 40.dp.toPx() -> 1.0
        scale > 15.dp.toPx() -> 5.0
        else -> 10.0
    }
    val first = floor((position - centre / scale) / step) * step
    val last = ceil((position + centre / scale) / step) * step
    var tick = max(0.0, first)
    val label = TextStyle(color = Palette.muted, fontSize = 9.sp)
    while (tick <= last) {
        val at = centre + ((tick - position) * scale).toFloat()
        drawLine(Palette.muted, Offset(at, top - 6.dp.toPx()), Offset(at, top), strokeWidth = 1f)
        val seconds = tick.toLong()
        drawText(measurer, "%d:%02d".format(seconds / 60, seconds % 60), Offset(at + 3f, 2f), label)
        tick += step
    }
}

/** One clip on its lane: its colour, its name, and handles when selected. */
private fun DrawScope.drawClip(
    measurer: TextMeasurer,
    clip: Clip,
    left: Float,
    right: Float,
    laneTop: Float,
    laneHeight: Float,
    alpha: Float,
    selected: Boolean,
) {
    if (right < 0 || left > size.width) return
    val corner = CornerRadius(6.dp.toPx())
    val box = Size(max(2f, right - left - 2f), laneHeight - 8f)
    val origin = Offset(left + 1f, laneTop + 4f)
    drawRoundRect(Palette.clip(clip.kind).copy(alpha = alpha), origin, box, corner)
    if (clip.videoEffects.isNotEmpty() && clip.kind != ClipKind.Transition) {
        // A dot: this clip carries effects.
        drawCircle(Palette.accent, 3.dp.toPx(), Offset(origin.x + box.width - 8.dp.toPx(), origin.y + box.height - 8.dp.toPx()))
    }
    if (selected) {
        drawRoundRect(Color.White, origin, box, corner, style = Stroke(width = 2.dp.toPx()))
        // Trim handles at both ends.
        val handle = Size(8.dp.toPx(), box.height)
        drawRoundRect(Color.White, origin, handle, corner)
        drawRoundRect(Color.White, Offset(origin.x + box.width - handle.width, origin.y), handle, corner)
        val grip = Palette.background
        val mid = origin.y + box.height / 2f
        drawLine(grip, Offset(origin.x + handle.width / 2f, mid - 6.dp.toPx()), Offset(origin.x + handle.width / 2f, mid + 6.dp.toPx()), 1.5.dp.toPx())
        val rightGrip = origin.x + box.width - handle.width / 2f
        drawLine(grip, Offset(rightGrip, mid - 6.dp.toPx()), Offset(rightGrip, mid + 6.dp.toPx()), 1.5.dp.toPx())
    }
    if (box.width > 24.dp.toPx()) {
        clipRect(origin.x, origin.y, origin.x + box.width, origin.y + box.height) {
            val text = clip.name.ifBlank { clip.kind.name }
            val laid = measurer.measure(
                text,
                TextStyle(color = Color.White.copy(alpha = max(alpha, 0.5f)), fontSize = 11.sp),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                constraints = Constraints(maxWidth = max(1, (box.width - 24.dp.toPx()).toInt())),
            )
            drawText(laid, topLeft = Offset(max(origin.x, 0f) + 12.dp.toPx(), origin.y + 4.dp.toPx()))
        }
    }
}
