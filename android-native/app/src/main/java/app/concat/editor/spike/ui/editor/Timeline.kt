// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.concat.editor.spike.api.ProjectView
import app.concat.editor.spike.ui.Palette
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/**
 * The timeline: the playhead fixed at the centre and the edit sliding under
 * it. A drag scrubs, a pinch zooms, a tap selects the clip under the finger
 * (or nothing). Lanes are the timeline's tracks, top to bottom.
 */
@Composable
fun Timeline(
    view: ProjectView,
    position: Double,
    selected: String?,
    onScrub: (Double) -> Unit,
    onSelect: (String?) -> Unit,
    modifier: Modifier = Modifier,
) {
    var pxPerSecond by remember { mutableFloatStateOf(0f) }
    val measurer = rememberTextMeasurer()
    val currentView by rememberUpdatedState(view)
    val currentPosition by rememberUpdatedState(position)
    val scrub by rememberUpdatedState(onScrub)
    val choose by rememberUpdatedState(onSelect)
    val ruler = 22.dp
    val laneMax = 52.dp

    Canvas(
        modifier
            .fillMaxSize()
            .pointerInput(Unit) {
                detectTransformGestures { _, pan, zoom, _ ->
                    val scale = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
                    pxPerSecond = (scale * zoom).coerceIn(8.dp.toPx(), 600.dp.toPx())
                    if (pan.x != 0f) scrub(currentPosition - pan.x / pxPerSecond)
                }
            }
            .pointerInput(Unit) {
                detectTapGestures { tap ->
                    val scale = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
                    val lanes = max(1, currentView.tracks.size)
                    val laneHeight = min(laneMax.toPx(), (size.height - ruler.toPx()) / lanes)
                    val lane = floor((tap.y - ruler.toPx()) / laneHeight).toInt()
                    val time = currentPosition + (tap.x - size.width / 2f) / scale
                    val track = currentView.tracks.getOrNull(lane)
                    val hit = currentView.clips.firstOrNull { it.trackId == track?.id && time >= it.start && time < it.end }
                    choose(hit?.id)
                }
            },
    ) {
        val scale = if (pxPerSecond <= 0f) 64.dp.toPx() else pxPerSecond
        val centre = size.width / 2f
        val top = ruler.toPx()
        val lanes = max(1, view.tracks.size)
        val laneHeight = min(laneMax.toPx(), (size.height - top) / lanes)
        fun x(seconds: Double) = centre + ((seconds - position) * scale).toFloat()

        drawRect(Palette.background)

        // The ruler: a tick a second (or wider apart when zoomed out), labelled.
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
            val at = x(tick)
            drawLine(Palette.muted, Offset(at, top - 6.dp.toPx()), Offset(at, top), strokeWidth = 1f)
            val seconds = tick.toLong()
            drawText(measurer, "%d:%02d".format(seconds / 60, seconds % 60), Offset(at + 3f, 2f), label)
            tick += step
        }

        // Lanes, then each clip on its lane.
        val name = TextStyle(color = Color.White, fontSize = 11.sp)
        view.tracks.forEachIndexed { index, track ->
            val laneTop = top + index * laneHeight
            drawRect(Palette.surface, Offset(0f, laneTop + 2f), Size(size.width, laneHeight - 4f))
            for (clip in view.clips) {
                if (clip.trackId != track.id) continue
                val left = x(clip.start)
                val right = x(clip.end)
                if (right < 0 || left > size.width) continue
                val corner = CornerRadius(6.dp.toPx())
                val box = Size(max(2f, right - left - 2f), laneHeight - 8f)
                val origin = Offset(left + 1f, laneTop + 4f)
                drawRoundRect(Palette.clip(clip.kind).copy(alpha = if (track.visible) 1f else 0.4f), origin, box, corner)
                if (clip.id == selected) {
                    drawRoundRect(Color.White, origin, box, corner, style = Stroke(width = 2.dp.toPx()))
                }
                if (box.width > 24.dp.toPx()) {
                    clipRect(origin.x, origin.y, origin.x + box.width, origin.y + box.height) {
                        val text = clip.name.ifBlank { clip.kind.name }
                        val laid = measurer.measure(
                            text,
                            name,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            constraints = Constraints(maxWidth = max(1, (box.width - 12.dp.toPx()).toInt())),
                        )
                        drawText(laid, topLeft = Offset(max(origin.x, 0f) + 6.dp.toPx(), origin.y + 4.dp.toPx()))
                    }
                }
            }
        }
        if (view.clips.isEmpty()) {
            drawText(
                measurer,
                "Add media to start",
                Offset(centre + 12.dp.toPx(), top + laneHeight / 2f - 8.dp.toPx()),
                TextStyle(color = Palette.muted, fontSize = 13.sp),
            )
        }

        // The playhead.
        drawLine(Color.White, Offset(centre, 0f), Offset(centre, size.height), strokeWidth = 2.dp.toPx())
        drawCircle(Color.White, 4.dp.toPx(), Offset(centre, 4.dp.toPx()))
    }
}
