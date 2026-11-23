// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FloatExponentialDecaySpec
import androidx.compose.animation.core.animateDecay
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.vector.VectorPainter
import androidx.compose.ui.graphics.vector.rememberVectorPainter
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
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.concat.editor.spike.api.Clip
import app.concat.editor.spike.api.ClipKind
import app.concat.editor.spike.api.ProjectView
import app.concat.editor.spike.media.MediaCache
import app.concat.editor.spike.media.StripKey
import app.concat.editor.spike.ui.icons.ConcatIcons
import app.concat.editor.spike.ui.secondsLabel
import app.concat.editor.spike.ui.theme.ConcatColors
import app.concat.editor.spike.ui.theme.Dimens
import app.concat.editor.spike.ui.theme.Inter
import app.concat.editor.spike.ui.theme.KindColors
import app.concat.editor.spike.ui.theme.Motion
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
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

/** One lane on screen: which track, where, how tall, and whether it is the main one. */
private class LaneRow(val trackIndex: Int, val trackId: String, val top: Float, val height: Float, val main: Boolean) {
    val bottom get() = top + height
    val centre get() = top + height / 2f
}

/**
 * The lanes, top first, over the document's tracks, which are stored
 * bottom first. The main track is the lowest lane holding pictures (the
 * first when none does): 56dp and filmstripped; the lanes above it are
 * overlays at 32dp; an empty lane is a thin 20dp slot to drop into.
 */
private class LaneLayout(val rows: List<LaneRow>, val gap: Float) {
    val top get() = rows.firstOrNull()?.top ?: 0f
    val bottom get() = rows.lastOrNull()?.bottom ?: 0f
    val main get() = rows.firstOrNull { it.main }

    /** The row under [y]: -1 above them all, `rows.size` below. */
    fun rowAt(y: Float): Int {
        if (rows.isEmpty()) return 0
        if (y < top - gap) return -1
        rows.forEachIndexed { i, row -> if (y < row.bottom + gap / 2f) return i }
        return if (y > bottom + gap * 2 + 12f) rows.size else rows.size - 1
    }

    fun rowOfTrack(trackId: String) = rows.indexOfFirst { it.trackId == trackId }
}

private const val MIN_LENGTH = 1.0 / 60

/** The main track: the lowest lane with a picture on it, else the lowest lane. */
fun mainTrackIndex(view: ProjectView): Int {
    val pictures = view.clips.filter { it.kind == ClipKind.Video || it.kind == ClipKind.Image }.map { it.trackId }.toSet()
    return view.tracks.indexOfFirst { it.id in pictures }.takeIf { it >= 0 } ?: 0
}

/** The glyphs the timeline paints, as painters it can draw in a DrawScope. */
private class TimelineGlyphs(
    val text: VectorPainter,
    val music: VectorPainter,
    val image: VectorPainter,
    val film: VectorPainter,
    val transition: VectorPainter,
    val sparkle: VectorPainter,
    val plus: VectorPainter,
    val volume: VectorPainter,
    val volumeOff: VectorPainter,
    val filter: VectorPainter,
    val shapes: VectorPainter,
)

/**
 * The timeline: the playhead fixed at the centre and the edit sliding under
 * it. A drag scrubs and flings, a pinch zooms, a tap selects the clip under
 * the finger (or nothing). A long press on a clip picks it up to move it,
 * onto another lane or past the top or bottom one for a new lane; the
 * selected clip's handles trim its ends. Moves and trims snap to the
 * playhead and to the other clips' edges, with a tick of haptics.
 *
 * The main track is tiled with filmstrips (`media.filmstrip`), sound with
 * its waveform (`media.peaks`); a cover square and the track's sound switch
 * sit before it, a "+" after it, and a transition button on every cut.
 */
@Composable
fun Timeline(
    view: ProjectView,
    projectPath: String,
    media: MediaCache,
    position: Double,
    selected: String?,
    onScrub: (Double) -> Unit,
    onSelect: (String?) -> Unit,
    onMove: (clipId: String, start: Double, target: LaneTarget) -> Unit,
    onTrim: (clipId: String, edge: String, delta: Double) -> Unit,
    onAddMedia: () -> Unit,
    onTransition: (nextClipId: String) -> Unit,
    onMuteTrack: (trackId: String, muted: Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    var pxPerSecond by remember { mutableFloatStateOf(0f) }
    var drag by remember { mutableStateOf<DragPreview?>(null) }
    var fling by remember { mutableStateOf<Job?>(null) }
    /** Vertical scroll when the lanes overflow; null follows the main track. */
    var scrollY by remember { mutableStateOf<Float?>(null) }
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
    val addMedia by rememberUpdatedState(onAddMedia)
    val transition by rememberUpdatedState(onTransition)
    val muteTrack by rememberUpdatedState(onMuteTrack)

    // The selection fades in; the playhead's knob springs while a finger is down.
    val selection = remember { Animatable(1f) }
    LaunchedEffect(selected) {
        if (selected != null) {
            selection.snapTo(0f)
            selection.animateTo(1f, Motion.tweenShort())
        }
    }
    val knob = remember { Animatable(0f) }

    val glyphs = TimelineGlyphs(
        text = rememberVectorPainter(ConcatIcons.TextMark),
        music = rememberVectorPainter(ConcatIcons.Music),
        image = rememberVectorPainter(ConcatIcons.Image),
        film = rememberVectorPainter(ConcatIcons.Film),
        transition = rememberVectorPainter(ConcatIcons.Transition),
        sparkle = rememberVectorPainter(ConcatIcons.Sparkle),
        plus = rememberVectorPainter(ConcatIcons.Plus),
        volume = rememberVectorPainter(ConcatIcons.Volume),
        volumeOff = rememberVectorPainter(ConcatIcons.VolumeOff),
        filter = rememberVectorPainter(ConcatIcons.Filter),
        shapes = rememberVectorPainter(ConcatIcons.Shapes),
    )

    Canvas(
        modifier
            .fillMaxSize()
            .pointerInput(Unit) {
                fun scaleNow() = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
                val snapPx = 10.dp.toPx()
                val handlePx = 16.dp.toPx()
                val handleWidth = HANDLE.toPx()

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
                    scope.launch { knob.animateTo(1f, Motion.knob()) }
                    try {
                        val view = currentView
                        val scale = scaleNow()
                        val playhead = currentPosition
                        val layout = layoutLanes(view, size.height.toFloat(), scrollY, this)
                        val centre = size.width / 2f
                        fun timeAt(x: Float) = playhead + (x - centre) / scale
                        fun xOf(t: Double) = centre + ((t - playhead) * scale).toFloat()
                        val downRowIndex = layout.rowAt(down.position.y)
                        val downRow = layout.rows.getOrNull(downRowIndex)
                        val downTime = timeAt(down.position.x)
                        val hit = view.clips.firstOrNull {
                            it.trackId == downRow?.trackId && downTime >= it.start && downTime < it.end &&
                                down.position.y >= downRow.top && down.position.y <= downRow.bottom
                        }

                        // A handle of the selected clip: trim that end.
                        val chosen = view.clip(currentSelected)
                        val edge = chosen?.let { clip ->
                            val row = layout.rows.getOrNull(layout.rowOfTrack(clip.trackId)) ?: return@let null
                            val inLane = down.position.y >= row.top - 6f && down.position.y <= row.bottom + 6f
                            val left = xOf(clip.start)
                            val right = xOf(clip.end)
                            when {
                                !inLane -> null
                                down.position.x in (left - handleWidth - handlePx / 2)..(left + handlePx / 2) &&
                                    down.position.x < (left + right) / 2 -> "start"
                                down.position.x in (right - handlePx / 2)..(right + handleWidth + handlePx / 2) -> "end"
                                else -> null
                            }
                        }
                        if (chosen != null && edge != null) {
                            down.consume()
                            val row = layout.rowOfTrack(chosen.trackId)
                            val (low, high) = trimRange(view, chosen, edge)
                            var delta = 0.0
                            var lastSnap: Double? = null
                            var atLimit = false
                            while (true) {
                                val event = awaitPointerEvent()
                                val change = event.changes.firstOrNull { it.id == down.id } ?: break
                                if (!change.pressed) break
                                change.consume()
                                val wanted = ((change.position.x - down.position.x) / scale).toDouble()
                                val raw = wanted.coerceIn(low, high)
                                val limited = wanted != raw
                                if (limited && !atLimit) haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                atLimit = limited
                                val moving = if (edge == "start") chosen.start + raw else chosen.end + raw
                                val snapped = snap(listOf(moving), chosen.id, scale, playhead)
                                if (snapped != null && snapped.first != lastSnap) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                lastSnap = snapped?.first
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
                        var vertical = false
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
                                val moved = change.position - down.position
                                if (moved.getDistance() > slop) {
                                    outcome = Outcome.Pan
                                    vertical = abs(moved.y) > abs(moved.x) * 1.4f
                                    break
                                }
                            }
                        }

                        if (outcome == Outcome.Tap) {
                            val tapped = buttonAt(view, layout, down.position, ::xOf, this)
                            when (tapped) {
                                is TimelineButton.Add -> addMedia()
                                is TimelineButton.Cut -> transition(tapped.nextClipId)
                                is TimelineButton.Mute -> muteTrack(tapped.trackId, tapped.mute)
                                TimelineButton.Cover -> scrub(0.0)
                                null -> choose(if (hit?.id == currentSelected) null else hit?.id)
                            }
                            return@awaitEachGesture
                        }

                        if (outcome == Outcome.LongPress && hit != null) {
                            // Picked up: follow the finger, then drop.
                            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            choose(hit.id)
                            val grab = downTime - hit.start
                            var start = hit.start
                            var row = layout.rowOfTrack(hit.trackId)
                            var lastSnap: Double? = null
                            drag = DragPreview(hit.id, start, hit.duration, row, null)
                            while (true) {
                                val event = awaitPointerEvent()
                                val change = event.changes.firstOrNull { it.id == down.id } ?: break
                                if (!change.pressed) break
                                change.consume()
                                val wanted = max(0.0, timeAt(change.position.x) - grab)
                                val snapped = snap(listOf(wanted, wanted + hit.duration), hit.id, scale, playhead)
                                if (snapped != null && snapped.first != lastSnap) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                lastSnap = snapped?.first
                                start = when {
                                    snapped == null -> wanted
                                    snapped.second == 0 -> snapped.first
                                    else -> snapped.first - hit.duration
                                }.coerceAtLeast(0.0)
                                row = layout.rowAt(change.position.y).coerceIn(-1, layout.rows.size)
                                drag = DragPreview(hit.id, start, hit.duration, row, snapped?.first)
                            }
                            drag = null
                            val target = when {
                                row < 0 -> LaneTarget.NewTop
                                row >= layout.rows.size -> LaneTarget.NewBottom
                                else -> LaneTarget.Existing(layout.rows[row].trackId)
                            }
                            val sameLane = target is LaneTarget.Existing && target.trackId == hit.trackId
                            if (!sameLane || abs(start - hit.start) > 1e-4) move(hit.id, start, target)
                            return@awaitEachGesture
                        }

                        if (vertical) {
                            // Lanes taller than the space: scroll them.
                            val overflow = max(0f, layout.bottom - layout.top - (size.height - RULER.toPx() - 16.dp.toPx()))
                            var offset = scrollY ?: autoScroll(layout, size.height.toFloat(), this)
                            while (true) {
                                val event = awaitPointerEvent()
                                val change = event.changes.firstOrNull { it.id == down.id } ?: break
                                if (!change.pressed) break
                                offset = (offset - (change.position.y - change.previousPosition.y)).coerceIn(0f, overflow)
                                scrollY = offset
                                change.consume()
                            }
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
                    } finally {
                        scope.launch { knob.animateTo(0f, Motion.knob()) }
                    }
                }
            },
    ) {
        val scale = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
        val centre = size.width / 2f
        val layout = layoutLanes(view, size.height, scrollY, this)
        fun x(seconds: Double) = centre + ((seconds - position) * scale).toFloat()
        // Read so a strip or an outline landing draws the timeline again.
        media.version

        drawRect(ConcatColors.page)
        val main = layout.main
        val moving = drag

        clipRect(top = RULER.toPx()) {
            // Lanes, then their clips.
            for (row in layout.rows) {
                drawRoundRect(
                    if (row.main) ConcatColors.well else ConcatColors.launchPanel.copy(alpha = 0.6f),
                    Offset(0f, row.top),
                    Size(size.width, row.height),
                    CornerRadius(0f),
                )
            }
            for (row in layout.rows) {
                for (clip in view.clips) {
                    if (clip.trackId != row.trackId) continue
                    val beingMoved = moving?.clipId == clip.id
                    drawClip(
                        measurer, glyphs, media, projectPath, view, clip, x(clip.start), x(clip.end), row,
                        alpha = if (beingMoved) 0.3f else 1f,
                    )
                }
            }
            if (main != null) drawMainExtras(measurer, glyphs, media, view, main, selected, ::x)

            // The selection: a thick lime frame and chunky white handles.
            val chosen = view.clip(selected)
            if (chosen != null && moving?.clipId != chosen.id) {
                layout.rows.getOrNull(layout.rowOfTrack(chosen.trackId))?.let { row ->
                    drawSelection(measurer, chosen, x(chosen.start), x(chosen.end), row, selection.value)
                }
            }

            if (moving != null) {
                val clip = view.clip(moving.clipId)
                if (clip != null) {
                    val gap = layout.gap
                    val row = when {
                        moving.row < 0 -> LaneRow(-1, "", layout.top - gap - Dimens.overlayLane.toPx(), Dimens.overlayLane.toPx(), false)
                        moving.row >= layout.rows.size -> LaneRow(-1, "", layout.bottom + gap, Dimens.overlayLane.toPx(), false)
                        else -> layout.rows[moving.row]
                    }
                    if (moving.row < 0 || moving.row >= layout.rows.size) {
                        // A new lane is made where it drops.
                        val y = if (moving.row < 0) layout.top - gap / 2f else layout.bottom + gap / 2f
                        drawLine(
                            ConcatColors.accent, Offset(0f, y), Offset(size.width, y), strokeWidth = 2.dp.toPx(),
                            pathEffect = PathEffect.dashPathEffect(floatArrayOf(12f, 8f)),
                        )
                    }
                    val left = x(moving.start)
                    val right = x(moving.start + moving.duration)
                    drawClip(measurer, glyphs, media, projectPath, view, clip, left, right, row, alpha = 0.92f)
                    drawSelection(measurer, clip.copy(start = moving.start, duration = moving.duration), left, right, row, 1f)
                }
                moving.snappedAt?.let { at ->
                    drawLine(ConcatColors.accent, Offset(x(at), RULER.toPx()), Offset(x(at), size.height), strokeWidth = 1.dp.toPx())
                }
            }
        }

        drawRuler(measurer, position, scale, centre)

        // The playhead: a 2dp lime line with a knob that springs under a finger.
        val head = ConcatColors.accent
        val knobY = RULER.toPx() - 7.dp.toPx()
        drawLine(head, Offset(centre, knobY), Offset(centre, size.height), strokeWidth = 2.dp.toPx())
        val radius = (5 + 2.5f * knob.value).dp.toPx()
        drawCircle(head, radius, Offset(centre, knobY))
        drawCircle(ConcatColors.page, radius * 0.35f, Offset(centre, knobY))
    }
}

private val RULER = Dimens.ruler
private val SOUND_LANE = 40.dp
private val HANDLE = 14.dp
private val LANE_GAP = 6.dp

private enum class Outcome { Tap, Pan, LongPress }

/** The lanes laid out in [height], following the main track unless scrolled. */
private fun layoutLanes(view: ProjectView, height: Float, scrollY: Float?, density: androidx.compose.ui.unit.Density): LaneLayout =
    with(density) {
        val mainIndex = mainTrackIndex(view)
        val gap = LANE_GAP.toPx()
        val heights = view.tracks.indices.reversed().map { index ->
            val track = view.tracks[index]
            val used = view.clips.any { it.trackId == track.id }
            val sound = view.clips.any { it.trackId == track.id && it.kind == ClipKind.Audio }
            index to when {
                index == mainIndex -> Dimens.mainTrack.toPx()
                sound -> SOUND_LANE.toPx()
                used -> Dimens.overlayLane.toPx()
                else -> Dimens.emptyLane.toPx()
            }
        }
        val total = heights.sumOf { it.second.toDouble() }.toFloat() + gap * max(0, heights.size - 1)
        val available = height - RULER.toPx() - 16.dp.toPx()
        var y = RULER.toPx() + 10.dp.toPx()
        if (total <= available) {
            // Room for all: the block sits a little above the middle, as CapCut's does.
            y += (available - total) * 0.3f
        } else {
            val probe = LaneLayout(rowsFrom(view, heights, y, gap, mainIndex), gap)
            y -= scrollY ?: autoScroll(probe, height, density)
        }
        LaneLayout(rowsFrom(view, heights, y, gap, mainIndex), gap)
    }

private fun rowsFrom(view: ProjectView, heights: List<Pair<Int, Float>>, startY: Float, gap: Float, mainIndex: Int): List<LaneRow> {
    var y = startY
    return heights.map { (index, h) ->
        LaneRow(index, view.tracks[index].id, y, h, index == mainIndex).also { y += h + gap }
    }
}

/** How far to scroll overflowing lanes so the main track sits about halfway down. */
private fun autoScroll(layout: LaneLayout, height: Float, density: androidx.compose.ui.unit.Density): Float = with(density) {
    val main = layout.main ?: return 0f
    val overflow = max(0f, layout.bottom - layout.top - (height - RULER.toPx() - 16.dp.toPx()))
    (main.centre - height * 0.55f).coerceIn(0f, overflow)
}

/** A tappable mark on the timeline that is not a clip. */
private sealed interface TimelineButton {
    data object Add : TimelineButton
    data object Cover : TimelineButton
    data class Cut(val nextClipId: String) : TimelineButton
    data class Mute(val trackId: String, val mute: Boolean) : TimelineButton
}

private fun buttonAt(
    view: ProjectView,
    layout: LaneLayout,
    at: Offset,
    x: (Double) -> Float,
    density: androidx.compose.ui.unit.Density,
): TimelineButton? = with(density) {
    val main = layout.main ?: return null
    val reach = 10.dp.toPx()
    val clips = view.clips.filter { it.trackId == main.trackId }.sortedBy { it.start }
    // The transition buttons sit on the cuts, half over each clip.
    if (abs(at.y - main.centre) < CUT_BUTTON.toPx() / 2 + reach) {
        for (i in 0 until clips.size - 1) {
            val a = clips[i]
            val b = clips[i + 1]
            if (abs(a.end - b.start) > 1e-3) continue
            if (abs(at.x - x(b.start)) < CUT_BUTTON.toPx() / 2 + reach / 2) return TimelineButton.Cut(b.id)
        }
    }
    if (at.y < main.top - reach || at.y > main.bottom + reach) return null
    val end = clips.maxOfOrNull { it.end } ?: 0.0
    val plusLeft = x(end) + 12.dp.toPx()
    if (at.x in (plusLeft - reach)..(plusLeft + PLUS.toPx() + reach)) return TimelineButton.Add
    val coverRight = x(0.0) - 12.dp.toPx()
    val coverLeft = coverRight - main.height
    if (at.x in coverLeft..coverRight) return TimelineButton.Cover
    val muteRight = coverLeft - 10.dp.toPx()
    if (at.x in (muteRight - 40.dp.toPx())..muteRight) {
        val videos = clips.filter { it.kind == ClipKind.Video }
        if (videos.isNotEmpty()) return TimelineButton.Mute(main.trackId, mute = videos.any { !it.muted })
    }
    null
}

private val CUT_BUTTON = 22.dp
private val PLUS = 40.dp

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

private fun ruleStyle() = TextStyle(color = ConcatColors.fgDim, fontSize = 9.sp, fontFamily = Inter, fontFeatureSettings = "tnum")

/** Time along the top: a labelled tick every so often and dots between. */
private fun DrawScope.drawRuler(measurer: TextMeasurer, position: Double, scale: Float, centre: Float) {
    drawRect(ConcatColors.page, Offset.Zero, Size(size.width, RULER.toPx()))
    val step = when {
        scale > 90.dp.toPx() -> 1.0
        scale > 40.dp.toPx() -> 2.0
        scale > 15.dp.toPx() -> 5.0
        else -> 10.0
    }
    val first = floor((position - centre / scale) / step) * step
    val last = ceil((position + centre / scale) / step) * step
    var tick = max(0.0, first)
    val label = ruleStyle()
    val mid = 8.dp.toPx()
    while (tick <= last) {
        val at = centre + ((tick - position) * scale).toFloat()
        val seconds = tick.toLong()
        // Measured first and drawn only where it fits: drawText given a place
        // past the right edge asks for a negative width and throws.
        val text = measurer.measure("%02d:%02d".format(seconds / 60, seconds % 60), label)
        val x = at - text.size.width / 2f
        if (x >= 0f && x + text.size.width <= size.width && abs(at - centre) > text.size.width / 2f + 2.dp.toPx()) {
            drawText(text, topLeft = Offset(x, mid - text.size.height / 2f))
        }
        val half = centre + ((tick + step / 2 - position) * scale).toFloat()
        if (half in 0f..size.width) drawCircle(ConcatColors.fgDim, 1.2.dp.toPx(), Offset(half, mid))
        tick += step
    }
}

private fun DrawScope.glyph(painter: VectorPainter, topLeft: Offset, size: Float, tint: Color) {
    translate(topLeft.x, topLeft.y) {
        with(painter) { draw(Size(size, size), colorFilter = ColorFilter.tint(tint)) }
    }
}

private fun kindGlyph(glyphs: TimelineGlyphs, clip: Clip): VectorPainter = when (clip.kind) {
    ClipKind.Text -> glyphs.text
    ClipKind.Audio, ClipKind.SoundLayer -> glyphs.music
    ClipKind.Image -> glyphs.image
    ClipKind.Video -> glyphs.film
    ClipKind.Transition -> glyphs.transition
    ClipKind.Layer -> glyphs.filter
    ClipKind.Other -> glyphs.shapes
}

/** One clip on its lane: filmstrip, waveform or kind colour, with its badges. */
private fun DrawScope.drawClip(
    measurer: TextMeasurer,
    glyphs: TimelineGlyphs,
    media: MediaCache,
    projectPath: String,
    view: ProjectView,
    clip: Clip,
    left: Float,
    right: Float,
    row: LaneRow,
    alpha: Float,
) {
    if (right < 0 || left > size.width) return
    val kind = KindColors.of(clip.kind)
    val inset = if (row.main) 0f else 1.dp.toPx()
    val box = Rect(left + 1f, row.top + inset, max(left + 3f, right - 1f), row.bottom - inset)
    val corner = CornerRadius((if (row.main) 6 else 5).dp.toPx())
    val shape = Path().apply { addRoundRect(RoundRect(box, corner)) }
    val file = view.mediaOf(clip)?.path
    clipPath(shape) {
        when {
            (clip.kind == ClipKind.Video || clip.kind == ClipKind.Image) && file != null -> {
                drawRect(kind.well.copy(alpha = alpha), box.topLeft, box.size)
                drawFilmstrip(media, file, clip, box, alpha)
                if (!row.main) {
                    // An overlay's strip is darkened under its label.
                    drawRect(Color.Black.copy(alpha = 0.25f * alpha), box.topLeft, box.size)
                }
            }
            clip.kind == ClipKind.Audio && file != null -> {
                drawRect(kind.well.copy(alpha = alpha), box.topLeft, box.size)
                val wave = Rect(box.left, box.top + 13.dp.toPx(), box.right, box.bottom - 1.dp.toPx())
                drawWaveform(media, projectPath, file, clip, wave, kind.color.copy(alpha = alpha))
            }
            else -> {
                drawRect(kind.color.copy(alpha = 0.92f * alpha), box.topLeft, box.size)
            }
        }
    }

    val width = box.width
    val glyphSize = 14.dp.toPx()
    var labelLeft = box.left + 6.dp.toPx()
    val clipLeftOnScreen = max(box.left, 0f)
    if (!row.main && width > 22.dp.toPx()) {
        // A sound's name sits small above its waveform; the rest are centred.
        val sound = clip.kind == ClipKind.Audio
        val size = if (sound) 10.dp.toPx() else glyphSize
        val lineY = if (sound) box.top + 7.dp.toPx() else row.centre
        labelLeft = clipLeftOnScreen + 6.dp.toPx()
        glyph(kindGlyph(glyphs, clip), Offset(labelLeft, lineY - size / 2f), size, Color.White.copy(alpha = alpha))
        labelLeft += size + 4.dp.toPx()
        val room = (box.right - labelLeft - 6.dp.toPx()).toInt()
        if (room > 12.dp.toPx()) {
            val laid = measurer.measure(
                clip.name.ifBlank { clip.kind.name },
                TextStyle(color = Color.White.copy(alpha = if (sound) 0.85f * alpha else alpha), fontSize = if (sound) 9.sp else 11.sp, fontFamily = Inter),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                constraints = Constraints(maxWidth = room),
            )
            drawText(laid, topLeft = Offset(labelLeft, lineY - laid.size.height / 2f))
        }
    }

    // Badges along the top: speed when it is not 1x, muted, and the effects dot.
    var badgeX = box.left + 4.dp.toPx()
    val badgeY = box.top + 3.dp.toPx()
    if (row.main && abs(clip.speed - 1.0) > 1e-3 && width > 36.dp.toPx()) {
        badgeX += drawBadge(measurer, "%.1fx".format(clip.speed), Offset(max(badgeX, 4.dp.toPx()), badgeY)) + 4.dp.toPx()
    }
    if (row.main && clip.muted && width > 28.dp.toPx()) {
        val s = 12.dp.toPx()
        drawRoundRect(Color.Black.copy(alpha = 0.55f), Offset(max(badgeX, 4.dp.toPx()), badgeY), Size(s + 6.dp.toPx(), s + 4.dp.toPx()), CornerRadius(4.dp.toPx()))
        glyph(glyphs.volumeOff, Offset(max(badgeX, 4.dp.toPx()) + 3.dp.toPx(), badgeY + 2.dp.toPx()), s, Color.White)
    }
    if (clip.videoEffects.isNotEmpty() && clip.kind != ClipKind.Transition && width > 18.dp.toPx()) {
        drawCircle(ConcatColors.accent, 3.dp.toPx(), Offset(box.right - 7.dp.toPx(), box.top + 7.dp.toPx()))
    }
}

/** A small dark pill with white text; its width. */
private fun DrawScope.drawBadge(measurer: TextMeasurer, text: String, at: Offset, background: Color = Color.Black.copy(alpha = 0.6f)): Float {
    val laid = measurer.measure(text, TextStyle(color = Color.White, fontSize = 9.sp, fontFamily = Inter, fontFeatureSettings = "tnum"))
    val pad = 4.dp.toPx()
    val w = laid.size.width + pad * 2
    drawRoundRect(background, at, Size(w, laid.size.height + 2.dp.toPx()), CornerRadius(4.dp.toPx()))
    drawText(laid, topLeft = Offset(at.x + pad, at.y + 1.dp.toPx()))
    return w
}

/**
 * Square tiles of the clip's own frames, left to right at the lane's height,
 * each sampled from the strip where it falls in time.
 */
private fun DrawScope.drawFilmstrip(media: MediaCache, file: String, clip: Clip, box: Rect, alpha: Float) {
    val tile = box.height
    val tilesWanted = ceil(box.width / tile).toInt().coerceAtLeast(1)
    val count = MediaCache.bucket(tilesWanted).coerceAtMost(60)
    val from = if (clip.kind == ClipKind.Image) 0.0 else clip.sourceStart
    val to = if (clip.kind == ClipKind.Image) 0.0 else clip.sourceStart + clip.duration * clip.speed
    val key = StripKey(file, round2(from), round2(to), count, tile.roundToInt().coerceIn(16, 240))
    val strip: ImageBitmap = media.strip(key) ?: return
    val stripCount = max(1, (strip.width.toFloat() / max(1, strip.height) + 0.5f).toInt()).let { guess ->
        // The strip is `count` tiles wide; a nearest stand-in may hold another count.
        if (strip.width % count == 0 && strip.width / count > 0) count else guess
    }
    val srcTileWidth = strip.width / stripCount
    val srcSide = min(srcTileWidth, strip.height)
    val first = max(0, floor((0f - box.left) / tile).toInt())
    val last = min(tilesWanted - 1, ceil((size.width - box.left) / tile).toInt())
    for (i in first..last) {
        val dstLeft = box.left + i * tile
        val centreAt = ((i * tile + tile / 2f) / box.width).coerceIn(0f, 0.9999f)
        val j = (centreAt * stripCount).toInt().coerceIn(0, stripCount - 1)
        val srcX = j * srcTileWidth + (srcTileWidth - srcSide) / 2
        val srcY = (strip.height - srcSide) / 2
        drawImage(
            strip,
            srcOffset = IntOffset(srcX, srcY),
            srcSize = IntSize(srcSide, srcSide),
            dstOffset = IntOffset(dstLeft.roundToInt(), box.top.roundToInt()),
            dstSize = IntSize(ceil(tile).toInt() + 1, box.height.roundToInt()),
            alpha = alpha,
        )
    }
}

private fun round2(value: Double) = (value * 100).roundToInt() / 100.0

/** The clip's stretch of its file's loudness outline, mirrored about the lane's middle. */
private fun DrawScope.drawWaveform(media: MediaCache, projectPath: String, file: String, clip: Clip, box: Rect, colour: Color) {
    val peaks = media.peaks(projectPath, file) ?: return
    if (peaks.max.isEmpty()) return
    val mid = box.center.y
    val half = box.height * 0.42f
    val step = 2.dp.toPx()
    val scale = box.width / clip.duration.coerceAtLeast(1e-6)
    var x = max(box.left, 0f)
    val end = min(box.right, size.width)
    while (x < end) {
        val t = clip.sourceStart + (x - box.left) / scale * clip.speed
        val i = (t * peaks.perSecond).toInt()
        if (i in peaks.max.indices) {
            val top = mid - peaks.max[i].coerceIn(0f, 1f) * half
            val bottom = mid - peaks.min.getOrElse(i) { -peaks.max[i] }.coerceIn(-1f, 0f) * half
            drawLine(colour, Offset(x, top), Offset(x, max(bottom, top + 1f)), strokeWidth = step * 0.6f)
        }
        x += step
    }
}

/** The selected clip's frame: 3dp lime, white handles outside each end, and its length. */
private fun DrawScope.drawSelection(measurer: TextMeasurer, clip: Clip, left: Float, right: Float, row: LaneRow, alpha: Float) {
    if (right < -HANDLE.toPx() || left > size.width + HANDLE.toPx()) return
    val stroke = 3.dp.toPx()
    val top = row.top - stroke / 2f
    val bottom = row.bottom + stroke / 2f
    val lime = ConcatColors.accent.copy(alpha = alpha)
    drawRoundRect(
        lime,
        Offset(left, top),
        Size(max(2f, right - left), bottom - top),
        CornerRadius(2.dp.toPx()),
        style = Stroke(width = stroke),
    )
    val handle = HANDLE.toPx()
    val radius = CornerRadius(6.dp.toPx())
    val white = Color.White.copy(alpha = alpha)
    val grip = ConcatColors.page.copy(alpha = alpha)
    val height = bottom - top + stroke
    val y = top - stroke / 2f
    // Left handle: rounded on the outside.
    val leftHandle = Path().apply {
        addRoundRect(RoundRect(Rect(left - handle, y, left, y + height), topLeft = radius, bottomLeft = radius))
    }
    drawPath(leftHandle, white)
    val rightHandle = Path().apply {
        addRoundRect(RoundRect(Rect(right, y, right + handle, y + height), topRight = radius, bottomRight = radius))
    }
    drawPath(rightHandle, white)
    val gripHalf = min(8.dp.toPx(), height * 0.22f)
    val mid = row.centre
    drawLine(grip, Offset(left - handle / 2f, mid - gripHalf), Offset(left - handle / 2f, mid + gripHalf), 2.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
    drawLine(grip, Offset(right + handle / 2f, mid - gripHalf), Offset(right + handle / 2f, mid + gripHalf), 2.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)

    // The duration badge, at the bottom left inside the clip (on screen).
    if (right - left > 34.dp.toPx()) {
        val laid = measurer.measure(secondsLabel(clip.duration), TextStyle(color = Color.White, fontSize = 9.sp, fontFamily = Inter, fontFeatureSettings = "tnum"))
        val pad = 4.dp.toPx()
        val bx = max(left + 4.dp.toPx(), 4.dp.toPx()).coerceAtMost(right - laid.size.width - pad * 2 - 4.dp.toPx())
        val by = row.bottom - laid.size.height - 5.dp.toPx()
        if (row.height >= 40.dp.toPx()) {
            drawRoundRect(Color.Black.copy(alpha = 0.65f * alpha), Offset(bx, by), Size(laid.size.width + pad * 2, laid.size.height + 2.dp.toPx()), CornerRadius(4.dp.toPx()))
            drawText(laid, topLeft = Offset(bx + pad, by + 1.dp.toPx()), alpha = alpha)
        }
    }
}

/** Before the main track, its cover and its sound switch; after it, "+"; on each cut, a transition button. */
private fun DrawScope.drawMainExtras(
    measurer: TextMeasurer,
    glyphs: TimelineGlyphs,
    media: MediaCache,
    view: ProjectView,
    main: LaneRow,
    selected: String?,
    x: (Double) -> Float,
) {
    val clips = view.clips.filter { it.trackId == main.trackId }.sortedBy { it.start }

    // The cover: a square of the first frame, labelled.
    val coverRight = x(0.0) - 12.dp.toPx()
    val side = main.height
    val coverLeft = coverRight - side
    if (coverRight > 0) {
        val rect = Rect(coverLeft, main.top, coverRight, main.bottom)
        val shape = Path().apply { addRoundRect(RoundRect(rect, CornerRadius(6.dp.toPx()))) }
        clipPath(shape) {
            drawRect(ConcatColors.raised, rect.topLeft, rect.size)
            val first = clips.firstOrNull { it.kind == ClipKind.Video || it.kind == ClipKind.Image }
            val file = first?.let { view.mediaOf(it)?.path }
            if (first != null && file != null) drawFilmstrip(media, file, first.copy(duration = first.duration.coerceAtMost(0.5)), rect, 1f)
            drawRect(Color.Black.copy(alpha = 0.72f), Offset(rect.left, rect.bottom - 16.dp.toPx()), Size(rect.width, 16.dp.toPx()))
        }
        drawRoundRect(ConcatColors.lineStrong, rect.topLeft, rect.size, CornerRadius(6.dp.toPx()), style = Stroke(1.dp.toPx()))
        val laid = measurer.measure("Cover", TextStyle(color = Color.White, fontSize = 9.sp, fontFamily = Inter))
        drawText(laid, topLeft = Offset(rect.center.x - laid.size.width / 2f, rect.bottom - 8.dp.toPx() - laid.size.height / 2f))

        // The main track's own sound: on or off.
        val videos = clips.filter { it.kind == ClipKind.Video }
        if (videos.isNotEmpty()) {
            val muted = videos.all { it.muted }
            val r = 15.dp.toPx()
            val cx = coverLeft - 10.dp.toPx() - r
            if (cx + r > 0) {
                drawCircle(ConcatColors.raised, r, Offset(cx, main.centre - 6.dp.toPx()))
                val g = 16.dp.toPx()
                glyph(if (muted) glyphs.volumeOff else glyphs.volume, Offset(cx - g / 2, main.centre - 6.dp.toPx() - g / 2), g, if (muted) ConcatColors.fgMuted else ConcatColors.fg)
                val label = measurer.measure(if (muted) "Muted" else "Sound", TextStyle(color = ConcatColors.fgMuted, fontSize = 8.sp, fontFamily = Inter))
                drawText(label, topLeft = Offset(cx - label.size.width / 2f, main.centre + r - 4.dp.toPx()))
            }
        }
    }

    // A transition button on every cut between two clips that touch.
    val transitions = view.clips.filter { it.kind == ClipKind.Transition }
    for (i in 0 until clips.size - 1) {
        val a = clips[i]
        val b = clips[i + 1]
        if (abs(a.end - b.start) > 1e-3) continue
        if (a.id == selected || b.id == selected) continue
        val cx = x(b.start)
        if (cx < -20 || cx > size.width + 20) continue
        val s = CUT_BUTTON.toPx()
        val has = transitions.any { it.start <= b.start + 1e-3 && it.end >= b.start - 1e-3 }
        drawRoundRect(
            if (has) ConcatColors.accent else Color.White,
            Offset(cx - s / 2, main.centre - s / 2),
            Size(s, s),
            CornerRadius(5.dp.toPx()),
        )
        drawRoundRect(ConcatColors.page.copy(alpha = 0.5f), Offset(cx - s / 2, main.centre - s / 2), Size(s, s), CornerRadius(5.dp.toPx()), style = Stroke(1.dp.toPx()))
        val g = 14.dp.toPx()
        glyph(glyphs.transition, Offset(cx - g / 2, main.centre - g / 2), g, ConcatColors.onAccent)
    }

    // "+" after the last clip: more media.
    val end = clips.maxOfOrNull { it.end } ?: 0.0
    val plusLeft = x(end) + 12.dp.toPx()
    if (plusLeft < size.width) {
        val s = PLUS.toPx()
        drawRoundRect(Color.White, Offset(plusLeft, main.centre - s / 2), Size(s, s), CornerRadius(8.dp.toPx()))
        val g = 22.dp.toPx()
        glyph(glyphs.plus, Offset(plusLeft + (s - g) / 2, main.centre - g / 2), g, ConcatColors.page)
        if (clips.isEmpty()) {
            val hint = measurer.measure("Add media", TextStyle(color = ConcatColors.fgMuted, fontSize = 13.sp, fontFamily = Inter))
            val hx = plusLeft + s + 10.dp.toPx()
            if (hx + hint.size.width <= size.width) drawText(hint, topLeft = Offset(hx, main.centre - hint.size.height / 2f))
        }
    }
}
