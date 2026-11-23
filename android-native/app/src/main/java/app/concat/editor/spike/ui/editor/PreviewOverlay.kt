// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateRotation
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import app.concat.editor.spike.api.Clip
import app.concat.editor.spike.api.ClipKind
import app.concat.editor.spike.api.ProjectView
import app.concat.editor.spike.ui.icons.ConcatIcons
import app.concat.editor.spike.ui.theme.ConcatColors
import app.concat.editor.spike.ui.theme.Motion
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.sin

/** A picture's placement as concat-project keeps it: scale, offsets as fractions of the frame, clockwise degrees. */
data class Placement(val scale: Double, val offsetX: Double, val offsetY: Double, val rotation: Double) {
    companion object {
        fun of(clip: Clip) = Placement(clip.scale, clip.offsetX, clip.offsetY, clip.rotation)
    }
}

/** A clip's box on the frame, in pixels of it: centre, size, and turn. */
private class Box(val centre: Offset, val width: Float, val height: Float, val degrees: Float) {
    fun contains(point: Offset, slack: Float = 0f): Boolean {
        val local = unrotate(point)
        return abs(local.x) <= width / 2 + slack && abs(local.y) <= height / 2 + slack
    }

    /** [point] in the box's own frame, its centre the origin. */
    fun unrotate(point: Offset): Offset {
        val d = point - centre
        val r = -degrees * PI.toFloat() / 180f
        return Offset(d.x * cos(r) - d.y * sin(r), d.x * sin(r) + d.y * cos(r))
    }

    fun corner(sx: Float, sy: Float): Offset {
        val r = degrees * PI.toFloat() / 180f
        val x = sx * width / 2
        val y = sy * height / 2
        return centre + Offset(x * cos(r) - y * sin(r), x * sin(r) + y * cos(r))
    }

    val corners get() = listOf(corner(-1f, -1f), corner(1f, -1f), corner(1f, 1f), corner(-1f, 1f))
}

/**
 * Where [clip] is drawn on a [frame]-sized picture at [placement]: a title's
 * lines at its size, a picture fitted to the frame (by its own aspect when
 * the probe found one).
 */
private fun boxOf(view: ProjectView, clip: Clip, placement: Placement, frame: Size): Box {
    val centre = Offset(
        frame.width / 2f + (placement.offsetX * frame.width).toFloat(),
        frame.height / 2f + (placement.offsetY * frame.height).toFloat(),
    )
    val scale = placement.scale.toFloat()
    if (clip.kind == ClipKind.Text) {
        val style = clip.text
        val font = ((style?.optDouble("fontSize", 0.09) ?: 0.09).toFloat() * frame.height * scale).coerceAtLeast(6f)
        val lines = (style?.optString("content") ?: clip.name).lines().ifEmpty { listOf("") }
        val longest = lines.maxOf { it.length }.coerceAtLeast(1)
        val pad = font * 0.3f
        return Box(centre, longest * font * 0.5f + pad * 2, lines.size * font * 1.2f + pad * 2, placement.rotation.toFloat())
    }
    val media = view.mediaOf(clip)
    val aspect = if (media != null && media.width > 0 && media.height > 0) media.width.toFloat() / media.height else frame.width / frame.height
    val fitted = if (aspect > frame.width / frame.height) Size(frame.width, frame.width / aspect) else Size(frame.height * aspect, frame.height)
    return Box(centre, fitted.width * scale, fitted.height * scale, placement.rotation.toFloat())
}

private enum class Grip { None, Body, Corner }

/**
 * Editing on the preview: tap a title, picture or overlay to select it;
 * one finger drags it, two pinch and turn it, a corner handle scales and
 * turns it about its centre. It snaps to the frame's centre lines (and to
 * square turns) with guides and a tick; the safe area shows while it moves.
 * The change is drawn here as it happens and written once, as the gesture
 * ends, through [onCommit]. A double tap on a title edits its words.
 */
@Composable
fun PreviewOverlay(
    view: ProjectView,
    position: Double,
    selected: String?,
    mainTrackId: String?,
    onSelect: (String?) -> Unit,
    onCommit: (clipId: String, placement: Placement) -> Unit,
    onEditText: (clipId: String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = LocalHapticFeedback.current
    var live by remember { mutableStateOf<Pair<String, Placement>?>(null) }
    var guides by remember { mutableStateOf(Pair(false, false)) }
    var moving by remember { mutableStateOf(false) }
    val currentView by rememberUpdatedState(view)
    val currentPosition by rememberUpdatedState(position)
    val currentSelected by rememberUpdatedState(selected)
    val select by rememberUpdatedState(onSelect)
    val commit by rememberUpdatedState(onCommit)
    val editText by rememberUpdatedState(onEditText)
    val rotateGlyph = rememberVectorPainter(ConcatIcons.Rotate)
    val appear = remember { Animatable(0f) }
    LaunchedEffect(selected) {
        appear.snapTo(0f)
        if (selected != null) appear.animateTo(1f, Motion.tweenShort())
    }
    // The document caught up with what was drawn: let the live copy go.
    val committed = view.clip(live?.first)?.let { Placement.of(it) }
    LaunchedEffect(committed) { if (!moving) live = null }

    fun visible(clips: List<Clip>, at: Double) = clips.filter { it.placeable && at >= it.start && at < it.end }

    /** What a tap at [point] lands on: titles and overlays first, the main track's picture last. */
    fun hitTest(point: Offset, frame: Size): Clip? {
        val v = currentView
        val order = v.tracks.map { it.id }
        return visible(v.clips, currentPosition)
            .sortedWith(
                compareByDescending<Clip> { it.trackId != mainTrackId }
                    .thenByDescending { it.kind == ClipKind.Text }
                    .thenByDescending { order.indexOf(it.trackId) },
            )
            .firstOrNull { boxOf(v, it, Placement.of(it), frame).contains(point, 8f) }
    }

    var lastTap by remember { mutableStateOf(Pair("", 0L)) }

    Canvas(
        modifier.fillMaxSize().pointerInput(Unit) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                val frame = Size(size.width.toFloat(), size.height.toFloat())
                val v = currentView
                val chosen = v.clip(currentSelected)?.takeIf { it.placeable && currentPosition >= it.start && currentPosition < it.end }
                val handleReach = 24.dp.toPx()
                var grip = Grip.None
                var target: Clip? = null
                // A title or overlay under the finger wins over the main
                // picture behind it, even while that picture is selected.
                val hit = hitTest(down.position, frame)
                val over = hit?.takeIf { it.id != chosen?.id && it.trackId != mainTrackId }
                if (chosen != null) {
                    val box = boxOf(v, chosen, live?.takeIf { it.first == chosen.id }?.second ?: Placement.of(chosen), frame)
                    when {
                        box.corners.any { (it - down.position).getDistance() < handleReach } -> grip = Grip.Corner
                        over == null && box.contains(down.position, 6f) -> grip = Grip.Body
                    }
                    if (grip != Grip.None) target = chosen
                }
                if (target == null) {
                    target = hit
                    if (target != null) grip = Grip.Body
                }
                val clip = target
                if (clip == null) {
                    // Nothing here: a tap clears the selection.
                    var tapped = true
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.id == down.id } ?: break
                        if ((change.position - down.position).getDistance() > viewConfiguration.touchSlop) tapped = false
                        if (!change.pressed) break
                    }
                    if (tapped) select(null)
                    return@awaitEachGesture
                }
                down.consume()
                if (clip.id != currentSelected) select(clip.id)

                val start = live?.takeIf { it.first == clip.id }?.second ?: Placement.of(clip)
                var now = start
                val startBox = boxOf(v, clip, start, frame)
                val grabbed = down.position - startBox.centre
                var travelled = 0f
                var twoFingers = false
                val snapPx = 8.dp.toPx()
                var snapped = Pair(false, false)
                moving = true
                while (true) {
                    val event = awaitPointerEvent()
                    val pressed = event.changes.filter { it.pressed }
                    if (pressed.isEmpty()) break
                    if (pressed.size > 1) twoFingers = true
                    var scale = now.scale
                    var rotation = now.rotation
                    var centre: Offset
                    if (grip == Grip.Corner && !twoFingers) {
                        val change = event.changes.firstOrNull { it.id == down.id } ?: break
                        val from = down.position - startBox.centre
                        val to = change.position - startBox.centre
                        scale = (start.scale * to.getDistance() / max(1f, from.getDistance())).coerceIn(0.05, 8.0)
                        rotation = start.rotation + (atan2(to.y, to.x) - atan2(from.y, from.x)) * 180.0 / PI
                        centre = startBox.centre
                        travelled = max(travelled, (change.position - down.position).getDistance())
                    } else if (pressed.size > 1) {
                        scale = (now.scale * event.calculateZoom()).coerceIn(0.05, 8.0)
                        rotation = now.rotation + event.calculateRotation()
                        val pan = event.calculatePan()
                        centre = boxOf(v, clip, now, frame).centre + pan
                        travelled += pan.getDistance() + 20f
                    } else {
                        val change = event.changes.firstOrNull { it.id == down.id } ?: pressed.first()
                        centre = change.position - grabbed
                        travelled = max(travelled, (change.position - down.position).getDistance())
                    }
                    // Snap the centre to the frame's middle lines, and square turns.
                    val snapX = abs(centre.x - frame.width / 2) < snapPx
                    val snapY = abs(centre.y - frame.height / 2) < snapPx
                    if (snapX) centre = Offset(frame.width / 2, centre.y)
                    if (snapY) centre = Offset(centre.x, frame.height / 2)
                    val square = (rotation / 90.0).roundToInt() * 90.0
                    if (abs(rotation - square) < 4.0 && (grip == Grip.Corner || twoFingers)) rotation = square
                    if ((snapX && !snapped.first) || (snapY && !snapped.second)) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    snapped = Pair(snapX, snapY)
                    now = Placement(
                        scale,
                        ((centre.x - frame.width / 2) / frame.width).toDouble(),
                        ((centre.y - frame.height / 2) / frame.height).toDouble(),
                        rotation,
                    )
                    if (travelled > viewConfiguration.touchSlop) {
                        live = clip.id to now
                        guides = snapped
                    }
                    event.changes.forEach { it.consume() }
                }
                moving = false
                guides = Pair(false, false)
                if (travelled > viewConfiguration.touchSlop) {
                    live = clip.id to now
                    commit(clip.id, now)
                } else {
                    // A tap; two on one title edit its words.
                    val time = down.uptimeMillis
                    if (clip.kind == ClipKind.Text && lastTap.first == clip.id && time - lastTap.second < viewConfiguration.doubleTapTimeoutMillis + 100) {
                        editText(clip.id)
                        lastTap = Pair("", 0L)
                    } else {
                        lastTap = Pair(clip.id, time)
                    }
                }
            }
        },
    ) {
        val frame = size
        val stroke = 1.5.dp.toPx()
        if (moving) {
            // The safe area, and the centre guides when snapped.
            val insetX = frame.width * 0.06f
            val insetY = frame.height * 0.08f
            drawRect(
                Color.White.copy(alpha = 0.35f),
                Offset(insetX, insetY),
                Size(frame.width - insetX * 2, frame.height - insetY * 2),
                style = Stroke(1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(10f, 8f))),
            )
            if (guides.first) drawLine(ConcatColors.accent, Offset(frame.width / 2, 0f), Offset(frame.width / 2, frame.height), 1.5.dp.toPx())
            if (guides.second) drawLine(ConcatColors.accent, Offset(0f, frame.height / 2), Offset(frame.width, frame.height / 2), 1.5.dp.toPx())
        }
        val clip = view.clip(selected)?.takeIf { it.placeable && position >= it.start && position < it.end } ?: return@Canvas
        val placement = live?.takeIf { it.first == clip.id }?.second ?: Placement.of(clip)
        val box = boxOf(view, clip, placement, frame)
        val alpha = appear.value
        val outline = Path().apply {
            val c = box.corners
            moveTo(c[0].x, c[0].y)
            for (i in 1..3) lineTo(c[i].x, c[i].y)
            close()
        }
        drawPath(outline, Color.Black.copy(alpha = 0.35f * alpha), style = Stroke(stroke + 2.dp.toPx()))
        drawPath(outline, Color.White.copy(alpha = alpha), style = Stroke(stroke))
        val corners = box.corners
        for ((i, corner) in corners.withIndex()) {
            if (i == 2) continue
            drawCircle(Color.Black.copy(alpha = 0.3f * alpha), 7.dp.toPx(), corner)
            drawCircle(Color.White.copy(alpha = alpha), 6.dp.toPx(), corner)
        }
        // The bottom-right handle scales and turns: lime, with the turn mark.
        val handle = corners[2]
        val r = 12.dp.toPx()
        drawCircle(Color.Black.copy(alpha = 0.3f * alpha), r + 1.dp.toPx(), handle)
        drawCircle(ConcatColors.accent.copy(alpha = alpha), r, handle)
        val g = 14.dp.toPx()
        translate(handle.x - g / 2, handle.y - g / 2) {
            rotate(box.degrees, Offset(g / 2, g / 2)) {
                with(rotateGlyph) { draw(Size(g, g), alpha = alpha, colorFilter = ColorFilter.tint(ConcatColors.onAccent)) }
            }
        }
    }
}
