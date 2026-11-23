// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.concat.editor.spike.ui.icons.ConcatIcons
import app.concat.editor.spike.ui.theme.ConcatColors
import app.concat.editor.spike.ui.theme.ConcatType
import app.concat.editor.spike.ui.theme.Radii
import app.concat.editor.spike.ui.theme.Spacing

/** One of the desktop's glyphs, tinted. */
@Composable
fun ConcatIcon(icon: ImageVector, modifier: Modifier = Modifier, tint: Color = ConcatColors.fg, size: Dp = 22.dp, label: String? = null) {
    Icon(icon, contentDescription = label, tint = tint, modifier = modifier.size(size))
}

/** A round icon button: 40dp to touch, the glyph at 22. */
@Composable
fun IconAction(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    tint: Color = ConcatColors.fg,
    size: Dp = 22.dp,
    box: Dp = 40.dp,
) {
    Box(
        modifier
            .size(box)
            .clip(CircleShape)
            .clickable(enabled = enabled, role = Role.Button, onClickLabel = label, onClick = onClick)
            .alpha(if (enabled) 1f else 0.35f),
        contentAlignment = Alignment.Center,
    ) { ConcatIcon(icon, tint = tint, size = size, label = label) }
}

/** The primary action: solid lime, dark ink. */
@Composable
fun AccentButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: ImageVector? = null,
    height: Dp = 40.dp,
) {
    Row(
        modifier
            .height(height)
            .clip(RoundedCornerShape(height / 2))
            .background(if (enabled) ConcatColors.accent else ConcatColors.raised)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = Spacing.lg),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center,
    ) {
        val ink = if (enabled) ConcatColors.onAccent else ConcatColors.fgDim
        if (icon != null) {
            ConcatIcon(icon, tint = ink, size = 18.dp)
            Spacer(Modifier.width(Spacing.xs + 2.dp))
        }
        Text(text, style = ConcatType.label.copy(fontSize = 14.sp), color = ink, maxLines = 1)
    }
}

/** A quiet action: raised surface, light text. */
@Composable
fun GhostButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, icon: ImageVector? = null, height: Dp = 40.dp) {
    Row(
        modifier
            .height(height)
            .clip(RoundedCornerShape(height / 2))
            .background(ConcatColors.raised)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = Spacing.lg)
            .alpha(if (enabled) 1f else 0.4f),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center,
    ) {
        if (icon != null) {
            ConcatIcon(icon, tint = ConcatColors.fg, size = 18.dp)
            Spacer(Modifier.width(Spacing.xs + 2.dp))
        }
        Text(text, style = ConcatType.label.copy(fontSize = 14.sp), color = ConcatColors.fg, maxLines = 1)
    }
}

/** A choice among a few: lime outline and text when picked. */
@Composable
fun ChoiceChip(text: String, selected: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val shape = RoundedCornerShape(16.dp)
    Box(
        modifier
            .height(32.dp)
            .clip(shape)
            .background(if (selected) ConcatColors.accentSoft else ConcatColors.raised)
            .border(1.dp, if (selected) ConcatColors.accent else Color.Transparent, shape)
            .clickable(enabled = enabled, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = 14.dp)
            .alpha(if (enabled) 1f else 0.4f),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = ConcatType.label, color = if (selected) ConcatColors.accent else ConcatColors.fg, maxLines = 1)
    }
}

/** A labelled slider with its value on the right, lime on a dark rail. */
@Composable
fun ValueSlider(
    label: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    describe: (Float) -> String,
    onChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
    steps: Int = 0,
    onDone: () -> Unit = {},
) {
    Column(modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = ConcatType.label, color = ConcatColors.fgMuted, modifier = Modifier.weight(1f))
            Text(describe(value), style = ConcatType.timecode, color = ConcatColors.accent)
        }
        Slider(
            value = value.coerceIn(range),
            onValueChange = onChange,
            onValueChangeFinished = onDone,
            valueRange = range,
            steps = steps,
            colors = SliderDefaults.colors(
                thumbColor = ConcatColors.fgHot,
                activeTrackColor = ConcatColors.accent,
                inactiveTrackColor = ConcatColors.fieldActive,
                activeTickColor = ConcatColors.onAccent,
                inactiveTickColor = ConcatColors.fgDim,
            ),
            modifier = Modifier.height(36.dp),
        )
    }
}

/**
 * A tool's options, CapCut's way: a panel over the bottom of the timeline,
 * above the toolbar, so the preview stays in sight. A title row with a
 * close mark and (when it confirms) a check; the body scrolls when tall.
 */
@Composable
fun ToolPanel(
    title: String,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
    onConfirm: (() -> Unit)? = null,
    actions: @Composable () -> Unit = {},
    scroll: Boolean = true,
    padding: PaddingValues = PaddingValues(horizontal = Spacing.lg, vertical = Spacing.xs),
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(topStart = Radii.xl, topEnd = Radii.xl))
            .background(ConcatColors.launchPanel)
            // Taps on the panel stay on the panel, never reaching the timeline below.
            .clickable(remember { MutableInteractionSource() }, indication = null) {},
    ) {
        Box(Modifier.fillMaxWidth().padding(top = 6.dp), contentAlignment = Alignment.Center) {
            Box(Modifier.width(36.dp).height(4.dp).clip(CircleShape).background(ConcatColors.lineStrong))
        }
        Row(
            Modifier.fillMaxWidth().height(44.dp).padding(horizontal = Spacing.xs),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconAction(ConcatIcons.Close, "Close", onClose, size = 20.dp)
            Text(
                title,
                style = ConcatType.heading,
                color = ConcatColors.fg,
                modifier = Modifier.weight(1f).padding(horizontal = Spacing.xs),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            actions()
            if (onConfirm != null) {
                Box(
                    Modifier.padding(end = Spacing.xs).size(34.dp).clip(CircleShape).background(ConcatColors.accent)
                        .clickable(role = Role.Button, onClickLabel = "Done", onClick = onConfirm),
                    contentAlignment = Alignment.Center,
                ) { ConcatIcon(ConcatIcons.Check, tint = ConcatColors.onAccent, size = 20.dp, label = "Done") }
            }
        }
        // Weighted, so a body taller than the room left scrolls (or clips)
        // inside it rather than pushing the header out.
        val body = Modifier.weight(1f, fill = false).fillMaxWidth().padding(padding)
        Column(
            if (scroll) body.verticalScroll(rememberScrollState()) else body,
            verticalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            content()
            Spacer(Modifier.height(Spacing.sm))
        }
    }
}

/** One tool of the bottom toolbar: the glyph over its 11sp label. */
@Composable
fun ToolButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    active: Boolean = false,
    enabled: Boolean = true,
    width: Dp = 64.dp,
) {
    Column(
        modifier
            .widthIn(min = width)
            .fillMaxHeight()
            .clip(RoundedCornerShape(Radii.md))
            .clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = ripple(bounded = true, color = ConcatColors.accent),
                enabled = enabled,
                role = Role.Button,
                onClick = onClick,
            )
            .alpha(if (enabled) 1f else 0.35f)
            .padding(horizontal = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        ConcatIcon(icon, tint = if (active) ConcatColors.accent else ConcatColors.fg, size = 24.dp)
        Spacer(Modifier.height(6.dp))
        Text(
            label,
            style = ConcatType.caption,
            color = if (active) ConcatColors.accent else ConcatColors.fgMuted,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A small label in the panels: what the next control is. */
@Composable
fun PanelLabel(text: String, modifier: Modifier = Modifier) =
    Text(text, style = ConcatType.label, color = ConcatColors.fgMuted, modifier = modifier)
