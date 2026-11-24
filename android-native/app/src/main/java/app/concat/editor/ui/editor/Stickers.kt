// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui.editor

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.concat.editor.api.ShapeCard
import app.concat.editor.ui.components.PanelLabel
import app.concat.editor.ui.components.ToolPanel
import app.concat.editor.ui.theme.ConcatColors
import app.concat.editor.ui.theme.ConcatType
import app.concat.editor.ui.theme.Radii
import app.concat.editor.ui.theme.Spacing

/** How a card is named and drawn: the window's Stickers page, line for line (media-pane.slint). */
private class CardLook(val label: String, val path: String)

private val LOOKS = mapOf(
    "square" to CardLook("Square", "M4 4h16v16H4z"),
    "rounded" to CardLook("Rounded square", "M4 9a5 5 0 0 1 5-5h6a5 5 0 0 1 5 5v6a5 5 0 0 1-5 5H9a5 5 0 0 1-5-5z"),
    "circle" to CardLook("Circle", "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0"),
    "ring" to CardLook("Ring", "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0 M7 12a5 5 0 1 0 10 0a5 5 0 1 0 -10 0"),
    "triangle" to CardLook("Triangle", "M12 4l9 16H3z"),
    "pentagon" to CardLook("Pentagon", "M12 2.5l9.5 6.9-3.6 11.1H6.1L2.5 9.4z"),
    "polygon" to CardLook("Hexagon", "M12 2l8.66 5v10L12 22l-8.66-5V7z"),
    "star" to CardLook("Star", "M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01z"),
    "heart" to CardLook("Heart", "M12 20s-8-4.5-8-11a4.5 4.5 0 0 1 8-2.8A4.5 4.5 0 0 1 20 9c0 6.5-8 11-8 11z"),
    "pill" to CardLook("Pill", "M8 8h8a4 4 0 0 1 0 8H8a4 4 0 0 1 0-8z"),
    "parallelogram" to CardLook("Parallelogram", "M8 5h13l-5 14H3z"),
    "trapezoid" to CardLook("Trapezoid", "M8 5h8l5 14H3z"),
    "line" to CardLook("Line", "M3 12h18"),
    "arrow" to CardLook("Arrow", "M3 12h18 M15 6l6 6-6 6"),
    "double-arrow" to CardLook("Double arrow", "M3 12h18 M9 6l-6 6 6 6 M15 6l6 6-6 6"),
)

/** What the lane calls a card: its name here, or its id for one this build has no name for. */
fun stickerName(card: ShapeCard): String = LOOKS[card.id]?.label ?: card.id

/**
 * The Stickers page: every card the engine offers (`shapes.list`), in its
 * order. A tap lays the figure at the playhead over the picture, on the
 * first free lane above it, and selects it.
 */
@Composable
fun StickersPanel(cards: List<ShapeCard>, onPick: (ShapeCard) -> Unit, onClose: () -> Unit) {
    ToolPanel("Stickers", onClose = onClose, onConfirm = onClose) {
        if (cards.isEmpty()) {
            PanelLabel("The stickers are on their way.")
            return@ToolPanel
        }
        LazyRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            items(cards, key = { it.id }) { card -> StickerCard(card) { onPick(card) } }
        }
    }
}

@Composable
private fun StickerCard(card: ShapeCard, onClick: () -> Unit) {
    val look = LOOKS[card.id]
    val path = remember(card.id) { look?.let { PathParser().parsePathString(it.path).toPath() } }
    val tint = ConcatColors.fg
    Column(
        Modifier.width(64.dp).clip(RoundedCornerShape(Radii.md)).clickable(onClickLabel = "Add ${stickerName(card)}", onClick = onClick)
            .padding(Spacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier.size(52.dp).clip(RoundedCornerShape(Radii.sm)).background(ConcatColors.raised),
            contentAlignment = Alignment.Center,
        ) {
            if (path != null) {
                Canvas(Modifier.size(30.dp)) {
                    // The paths are drawn on a 24-unit square, as an icon is.
                    scale(size.width / 24f, size.height / 24f, pivot = androidx.compose.ui.geometry.Offset.Zero) {
                        drawPath(path, tint, style = Stroke(width = 1.8f, cap = StrokeCap.Round, join = StrokeJoin.Round))
                    }
                }
            }
        }
        Text(
            stickerName(card),
            style = ConcatType.caption,
            color = ConcatColors.fg,
            maxLines = 1,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}
