// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.concat.editor.spike.api.AppliedEffect
import app.concat.editor.spike.api.Clip
import app.concat.editor.spike.api.ClipKind
import app.concat.editor.spike.api.PackageInfo
import app.concat.editor.spike.api.ProjectView
import app.concat.editor.spike.ui.components.ChoiceChip
import app.concat.editor.spike.ui.components.ConcatIcon
import app.concat.editor.spike.ui.components.GhostButton
import app.concat.editor.spike.ui.components.PanelLabel
import app.concat.editor.spike.ui.components.ToolPanel
import app.concat.editor.spike.ui.components.ValueSlider
import app.concat.editor.spike.ui.icons.ConcatIcons
import app.concat.editor.spike.ui.theme.ConcatColors
import app.concat.editor.spike.ui.theme.ConcatType
import app.concat.editor.spike.ui.theme.Radii
import app.concat.editor.spike.ui.theme.Spacing
import kotlin.math.abs
import kotlin.math.ln
import kotlin.math.pow
import org.json.JSONObject

/*
 * The tools' sub-panels: what opens above the toolbar when a tool has
 * options, CapCut's way. Sliders apply when the finger lifts (one undo
 * step each), chips at once; the check closes the panel.
 */

private val SPEEDS = listOf(0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0)

/** Speed on a log rail from 0.1x to 10x, with the usual stops as chips. */
@Composable
fun SpeedPanel(clip: Clip, onApply: (Double) -> Unit, onClose: () -> Unit) {
    // The rail is log2 of the speed, so 1x sits in the middle.
    var rail by remember(clip.id) { mutableFloatStateOf(log2(clip.speed).toFloat()) }
    ToolPanel("Speed", onClose = onClose, onConfirm = onClose) {
        ValueSlider(
            "Speed",
            rail,
            log2(0.1).toFloat()..log2(10.0).toFloat(),
            { "%.2fx".format(2.0.pow(it.toDouble())) },
            onChange = { rail = it },
            onDone = { onApply(roundSpeed(2.0.pow(rail.toDouble()))) },
        )
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            for (speed in SPEEDS) {
                ChoiceChip(
                    if (speed < 1) "${speed}x" else "${speed.toInt()}${if (speed % 1.0 != 0.0) ".5" else ""}x",
                    selected = abs(clip.speed - speed) < 1e-3,
                    onClick = {
                        rail = log2(speed).toFloat()
                        onApply(speed)
                    },
                )
            }
        }
        PanelLabel("Duration ${"%.1f".format(clip.duration)}s at ${"%.2f".format(clip.speed)}x")
    }
}

private fun log2(value: Double) = ln(value.coerceAtLeast(1e-3)) / ln(2.0)
private fun roundSpeed(value: Double) = (value * 100).toInt() / 100.0

/** The clip's gain, 0 to 200%. */
@Composable
fun VolumePanel(clip: Clip, onApply: (Double) -> Unit, onMute: () -> Unit, onClose: () -> Unit) {
    var value by remember(clip.id) { mutableFloatStateOf(clip.volume.toFloat()) }
    ToolPanel("Volume", onClose = onClose, onConfirm = onClose) {
        ValueSlider("Volume", value, 0f..2f, { "${(it * 100).toInt()}%" }, onChange = { value = it }, onDone = { onApply(value.toDouble()) })
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            for (preset in listOf(0.0, 0.5, 1.0, 1.5, 2.0)) {
                ChoiceChip("${(preset * 100).toInt()}%", selected = abs(clip.volume - preset) < 1e-3, onClick = {
                    value = preset.toFloat()
                    onApply(preset)
                })
            }
        }
        if (clip.kind == ClipKind.Video) {
            ChoiceChip(if (clip.muted) "Muted: tap for sound" else "Mute this clip", selected = clip.muted, onClick = onMute)
        }
    }
}

/** How see-through a picture or title is. */
@Composable
fun OpacityPanel(clip: Clip, onApply: (Double) -> Unit, onClose: () -> Unit) {
    var value by remember(clip.id) { mutableFloatStateOf(clip.opacity.toFloat()) }
    ToolPanel("Opacity", onClose = onClose, onConfirm = onClose) {
        ValueSlider("Opacity", value, 0f..1f, { "${(it * 100).toInt()}%" }, onChange = { value = it }, onDone = { onApply(value.toDouble()) })
    }
}

/** Sound ramps in and out. */
@Composable
fun FadePanel(clip: Clip, onApply: (Double, Double) -> Unit, onClose: () -> Unit) {
    val most = (clip.duration / 2).coerceIn(0.1, 5.0).toFloat()
    var fadeIn by remember(clip.id) { mutableFloatStateOf(clip.fadeIn.toFloat().coerceAtMost(most)) }
    var fadeOut by remember(clip.id) { mutableFloatStateOf(clip.fadeOut.toFloat().coerceAtMost(most)) }
    ToolPanel("Fade", onClose = onClose, onConfirm = onClose) {
        ValueSlider("Fade in", fadeIn, 0f..most, { "%.1fs".format(it) }, onChange = { fadeIn = it }, onDone = { onApply(fadeIn.toDouble(), fadeOut.toDouble()) })
        ValueSlider("Fade out", fadeOut, 0f..most, { "%.1fs".format(it) }, onChange = { fadeOut = it }, onDone = { onApply(fadeIn.toDouble(), fadeOut.toDouble()) })
    }
}

val TEXT_COLOURS = listOf("#ffffff", "#000000", "#cbf53f", "#ffd60a", "#ff9f0a", "#ff375f", "#bf5af2", "#0a84ff", "#30d158")

fun colourOf(hex: String): Color = try {
    Color(android.graphics.Color.parseColor(hex))
} catch (_: Exception) {
    Color.White
}

/**
 * A title's words and look: a new one (at the playhead) or [existing]
 * restyled. The words in a field that takes the keyboard at once.
 */
@Composable
fun TextPanel(existing: Clip?, onSave: (content: String, size: Double, colour: String, weight: Double) -> Unit, onClose: () -> Unit) {
    val style = existing?.text
    var content by remember(existing?.id) { mutableStateOf(style?.optString("content") ?: "") }
    // The document's size is a fraction of the frame's height.
    var size by remember(existing?.id) { mutableFloatStateOf((style?.optDouble("fontSize", 0.07) ?: 0.07).toFloat()) }
    var colour by remember(existing?.id) { mutableStateOf(style?.optString("color", "#ffffff") ?: "#ffffff") }
    var weight by remember(existing?.id) { mutableStateOf(style?.optDouble("fontWeight", 700.0) ?: 700.0) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(existing?.id) { if (existing == null) focus.requestFocus() }
    fun save() {
        if (content.isNotBlank()) onSave(content.trim(), size.toDouble(), colour, weight)
        onClose()
    }

    ToolPanel(if (existing == null) "Add text" else "Text", onClose = onClose, onConfirm = ::save) {
        BasicTextField(
            value = content,
            onValueChange = { content = it },
            textStyle = ConcatType.title.copy(color = ConcatColors.fg),
            cursorBrush = SolidColor(ConcatColors.accent),
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
            maxLines = 4,
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
            decorationBox = { field ->
                Box(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.md)).background(ConcatColors.launchField)
                        .padding(horizontal = Spacing.md, vertical = 10.dp),
                ) {
                    if (content.isEmpty()) Text("Enter text", style = ConcatType.title, color = ConcatColors.fgDim)
                    field()
                }
            },
        )
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            ChoiceChip("Regular", selected = weight < 600, onClick = { weight = 400.0 })
            ChoiceChip("Bold", selected = weight >= 600 && weight < 800, onClick = { weight = 700.0 })
            ChoiceChip("Black", selected = weight >= 800, onClick = { weight = 900.0 })
        }
        ValueSlider("Size", size, 0.03f..0.2f, { "${(it * 100).toInt()}" }, onChange = { size = it })
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            for (hex in TEXT_COLOURS) {
                val picked = hex.equals(colour, ignoreCase = true)
                Box(
                    Modifier.size(32.dp).clip(CircleShape)
                        .border(if (picked) 2.dp else 1.dp, if (picked) ConcatColors.accent else ConcatColors.lineStrong, CircleShape)
                        .padding(if (picked) 4.dp else 0.dp)
                        .clip(CircleShape)
                        .background(colourOf(hex))
                        .clickable { colour = hex },
                )
            }
        }
    }
}

/**
 * Effects, filters or transitions as a row of tiles over the clip, CapCut's
 * way: "None" first, a tap applies, and the newest of this shelf's links
 * gets its strength slider beneath. "All" opens the full catalogue sheet
 * with search, categories and every knob.
 */
@Composable
fun PackagePanel(
    shelf: Shelf,
    catalogue: CatalogueViewModel,
    editor: EditorViewModel,
    onMore: () -> Unit,
    onClose: () -> Unit,
) {
    val state by catalogue.ui.collectAsStateWithLifecycle()
    val editorUi by editor.ui.collectAsStateWithLifecycle()
    val clip = editorUi.view?.clip(editorUi.selected)
    val kinds = shelf.kinds(clip)
    LaunchedEffect(Unit) { catalogue.load() }
    val offered = state.packages.filter { it.kind in kinds }
    // The chain's links this shelf owns, as (field, index, link).
    val links = clip?.let { c ->
        buildList {
            if ("audio" in kinds) c.filters.forEachIndexed { i, link -> add(Triple("filters", i, link)) }
            c.videoEffects.forEachIndexed { i, link ->
                val kind = catalogue.find(link.id)?.kind
                if (c.kind == ClipKind.Transition || kind in kinds || (kind == null && "effect" in kinds)) add(Triple("videoEffects", i, link))
            }
        }
    }.orEmpty()
    val applied = links.map { it.third.id }.toSet()

    ToolPanel(
        shelf.title,
        onClose = onClose,
        onConfirm = onClose,
        actions = { GhostButton("All", onMore, icon = ConcatIcons.Search, height = 32.dp, modifier = Modifier.padding(end = Spacing.sm)) },
        padding = androidx.compose.foundation.layout.PaddingValues(vertical = Spacing.xs),
    ) {
        when {
            !state.loaded -> PanelLabel("Loading...", Modifier.padding(horizontal = Spacing.lg))
            state.error != null -> PanelLabel(state.error ?: "", Modifier.padding(horizontal = Spacing.lg))
            clip == null && shelf != Shelf.Transitions -> PanelLabel("Select a clip to apply one.", Modifier.padding(horizontal = Spacing.lg))
        }
        LazyRow(
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = Spacing.lg),
        ) {
            if (shelf != Shelf.Transitions) {
                item(key = "none") {
                    PackageTileSmall(
                        name = "None",
                        tint = null,
                        selected = links.isEmpty(),
                        enabled = clip != null && links.isNotEmpty(),
                    ) {
                        clip?.let { c -> editor.removeLinks(c.id, links.map { it.first to it.second }) }
                    }
                }
            }
            items(offered, key = { it.id }) { pkg ->
                PackageTileSmall(
                    name = pkg.name,
                    tint = tileColour(pkg),
                    selected = pkg.id in applied,
                    enabled = clip != null || shelf == Shelf.Transitions,
                ) { editor.applyPackage(pkg.kind, pkg.id, pkg.name) }
            }
        }
        // The newest link's strength, live as the finger moves.
        val newest = links.maxByOrNull { if (it.first == "videoEffects") 1000 + it.second else it.second }
        val pkg = newest?.let { catalogue.find(it.third.id) }
        val knob = pkg?.params?.firstOrNull { it.key == pkg.intensity } ?: pkg?.params?.firstOrNull { it.type == "float" || it.type == "int" }
        if (clip != null && newest != null && pkg != null && knob != null) {
            LinkStrength(clip, newest.first, newest.second, newest.third, pkg, knob, editor)
        }
    }
}

@Composable
private fun LinkStrength(
    clip: Clip,
    field: String,
    index: Int,
    link: AppliedEffect,
    pkg: PackageInfo,
    knob: app.concat.editor.spike.api.ParamInfo,
    editor: EditorViewModel,
) {
    var value by remember(clip.id, field, index, link.id) { mutableFloatStateOf((link.params[knob.key] ?: knob.default).toFloat()) }
    val low = knob.min.toFloat()
    val high = knob.max.toFloat().coerceAtLeast(low + 1e-3f)
    Column(Modifier.padding(horizontal = Spacing.lg)) {
        ValueSlider(
            "${pkg.name} · ${knob.label}",
            value,
            low..high,
            { if (knob.type == "int" || knob.step >= 1) "%.0f%s".format(it, knob.unit) else "%.2f%s".format(it, knob.unit) },
            onChange = {
                value = it
                editor.setLink(clip.id, field, index, link.withParam(knob.key, it.toDouble()))
            },
        )
    }
}

private fun tileColour(pkg: PackageInfo): Brush {
    val hue = (pkg.id.hashCode() and 0xff) * 360f / 256f
    return Brush.linearGradient(listOf(Color.hsv(hue, 0.5f, 0.7f), Color.hsv((hue + 40f) % 360f, 0.65f, 0.3f)))
}

@Composable
private fun PackageTileSmall(name: String, tint: Brush?, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    Column(Modifier.width(64.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        val shape = RoundedCornerShape(Radii.md)
        Box(
            Modifier.size(64.dp).clip(shape)
                .then(if (tint != null) Modifier.background(tint) else Modifier.background(ConcatColors.raised))
                .border(if (selected) 2.dp else 0.dp, if (selected) ConcatColors.accent else Color.Transparent, shape)
                .clickable(enabled = enabled, onClick = onClick),
            contentAlignment = Alignment.Center,
        ) {
            if (tint == null) {
                ConcatIcon(ConcatIcons.EyeOff, tint = ConcatColors.fgMuted, size = 22.dp)
            } else if (selected) {
                Box(Modifier.size(22.dp).clip(CircleShape).background(ConcatColors.accent), contentAlignment = Alignment.Center) {
                    ConcatIcon(ConcatIcons.Check, tint = ConcatColors.onAccent, size = 14.dp)
                }
            }
        }
        Spacer(Modifier.height(4.dp))
        Text(
            name,
            style = ConcatType.caption,
            color = if (selected) ConcatColors.accent else if (enabled) ConcatColors.fg else ConcatColors.fgDim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
        )
    }
}

/** The frame's shapes. */
enum class AspectChoice(val label: String, val width: Int, val height: Int) {
    Portrait("9:16", 1080, 1920),
    Landscape("16:9", 1920, 1080),
    Square("1:1", 1080, 1080),
    Feed("4:5", 1080, 1350),
    Classic("3:4", 1080, 1440),
    Cinema("21:9", 2520, 1080),
}

@Composable
fun AspectPanel(view: ProjectView, onPick: (AspectChoice) -> Unit, onClose: () -> Unit) {
    ToolPanel("Aspect ratio", onClose = onClose, onConfirm = onClose) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(Spacing.md)) {
            for (choice in AspectChoice.entries) {
                val picked = view.width * choice.height == view.height * choice.width
                Column(
                    Modifier.clip(RoundedCornerShape(Radii.md)).clickable { onPick(choice) }.padding(Spacing.xs),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    val tall = 44.dp
                    val ratio = choice.width.toFloat() / choice.height
                    val w = if (ratio >= 1) tall else tall * ratio
                    val h = if (ratio >= 1) tall / ratio else tall
                    Box(Modifier.size(52.dp), contentAlignment = Alignment.Center) {
                        Box(
                            Modifier.size(w, h).clip(RoundedCornerShape(4.dp))
                                .border(2.dp, if (picked) ConcatColors.accent else ConcatColors.fgMuted, RoundedCornerShape(4.dp)),
                        )
                    }
                    Text(choice.label, style = ConcatType.caption, color = if (picked) ConcatColors.accent else ConcatColors.fg)
                }
            }
        }
    }
}

private val BACKGROUNDS = listOf("#000000", "#ffffff", "#1e1e22", "#cbf53f", "#ff375f", "#0a84ff", "#ffd60a", "#bf5af2")

@Composable
fun BackgroundPanel(onPick: (JSONObject) -> Unit, onClose: () -> Unit) {
    ToolPanel("Background", onClose = onClose, onConfirm = onClose) {
        PanelLabel("Behind the main track, where the picture does not fill the frame.")
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Box(
                Modifier.size(40.dp).clip(RoundedCornerShape(Radii.sm)).background(ConcatColors.raised)
                    .clickable { onPick(JSONObject().put("kind", "blur").put("amount", 0.6)) },
                contentAlignment = Alignment.Center,
            ) { Text("Blur", style = ConcatType.caption, color = ConcatColors.fg) }
            for (hex in BACKGROUNDS) {
                Box(
                    Modifier.size(40.dp).clip(RoundedCornerShape(Radii.sm)).background(colourOf(hex))
                        .border(1.dp, ConcatColors.lineStrong, RoundedCornerShape(Radii.sm))
                        .clickable { onPick(JSONObject().put("kind", "color").put("color", hex)) },
                )
            }
        }
    }
}
