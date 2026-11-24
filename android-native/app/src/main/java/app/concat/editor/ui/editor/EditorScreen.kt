// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui.editor

import android.app.Application
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import app.concat.editor.api.Clip
import app.concat.editor.api.ClipKind
import app.concat.editor.api.ProjectView
import app.concat.editor.concat
import app.concat.editor.ui.components.AccentButton
import app.concat.editor.ui.components.ConcatIcon
import app.concat.editor.ui.components.IconAction
import app.concat.editor.ui.components.ToolButton
import app.concat.editor.ui.icons.ConcatIcons
import app.concat.editor.ui.shortTimecode
import app.concat.editor.ui.theme.ConcatColors
import app.concat.editor.ui.theme.ConcatType
import app.concat.editor.ui.theme.Dimens
import app.concat.editor.ui.theme.Motion
import app.concat.editor.ui.theme.Radii
import app.concat.editor.ui.theme.Spacing
import kotlinx.coroutines.delay

/** Which row of tools shows when nothing is selected. */
private enum class Menu { Root, Audio, Text }

/** A tool's options, open above the toolbar. */
private enum class Panel { Speed, Volume, Opacity, Fade, Crop, Mask, Animation, TextNew, TextEdit, Effects, Filters, Transitions, Aspect, Background, Captions, Speech, Voiceover, Stickers, Cover }

/** The panels that belong to the selected clip, closed when it is let go. */
private val CLIP_PANELS = setOf(Panel.Speed, Panel.Volume, Panel.Opacity, Panel.Fade, Panel.Crop, Panel.Mask, Panel.Animation, Panel.TextEdit, Panel.Effects, Panel.Filters)

/** One tool of the bottom toolbar. */
private class Tool(val icon: ImageVector, val label: String, val enabled: Boolean = true, val active: Boolean = false, val onClick: () -> Unit)

/**
 * The editor, CapCut's way: a top bar, the preview (at most about 42% of
 * the height: the timeline is the product), the transport, the timeline,
 * and a scrollable toolbar whose tools open their options in a panel
 * above it. [startTool] opens a panel at once ("captions", "speech").
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun EditorScreen(path: String, session: Int, onClosed: () -> Unit, startTool: String? = null) {
    val application = LocalContext.current.applicationContext as Application
    val model: EditorViewModel = viewModel(
        key = "$path#$session",
        factory = viewModelFactory { initializer { EditorViewModel(application, path) } },
    )
    // The panels' own models, one each per open project: their jobs outlive the panel.
    val captions: CaptionsViewModel = viewModel(
        key = "$path#$session#captions",
        factory = viewModelFactory { initializer { CaptionsViewModel(application, path) } },
    )
    val speech: SpeechViewModel = viewModel(
        key = "$path#$session#speech",
        factory = viewModelFactory { initializer { SpeechViewModel(application, path) } },
    )
    val voiceover: VoiceoverViewModel = viewModel(
        key = "$path#$session#voiceover",
        factory = viewModelFactory { initializer { VoiceoverViewModel(application) } },
    )
    val catalogue: CatalogueViewModel = viewModel(
        key = "catalogue",
        factory = viewModelFactory { initializer { CatalogueViewModel(application) } },
    )
    val ui by model.ui.collectAsStateWithLifecycle()
    val haptics = LocalHapticFeedback.current
    var menu by remember { mutableStateOf(Menu.Root) }
    var panel by remember {
        mutableStateOf(
            when (startTool) {
                "captions" -> Panel.Captions
                "speech" -> Panel.Speech
                else -> null
            },
        )
    }
    var shownPanel by remember { mutableStateOf(panel) }
    if (panel != null) shownPanel = panel
    var catalogueShelf by remember { mutableStateOf<Shelf?>(null) }
    var catalogueAdjust by remember { mutableStateOf(false) }
    var exporting by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var fullscreen by remember { mutableStateOf(false) }
    var inlineEdit by remember { mutableStateOf<String?>(null) }

    // Pause when the app is covered; save when it is no longer seen.
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, model) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_PAUSE -> model.pause()
                Lifecycle.Event.ON_STOP -> model.save()
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    BackHandler {
        when {
            inlineEdit != null -> inlineEdit = null
            fullscreen -> fullscreen = false
            panel != null -> panel = null
            ui.selected != null -> model.select(null)
            menu != Menu.Root -> menu = Menu.Root
            else -> model.close(onClosed)
        }
    }

    val pickVisual = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(50)) { uris ->
        model.import(uris, audio = false)
    }
    val pickOverlay = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(10)) { uris ->
        model.importOverlay(uris)
    }
    val pickAudio = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        model.import(uris, audio = true)
    }
    val pickReplacement = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        model.replace(uri)
    }
    val pickCover = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        model.setCoverPicture(uri)
    }
    val pickReplacementSound = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        model.replace(uri)
    }
    fun addMedia() = pickVisual.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))

    val view = ui.view
    val coverImage = remember(ui.cover) { ui.cover?.asImageBitmap() }
    val selected = view?.clip(ui.selected)
    val mainTrackId = view?.let { v -> v.tracks.getOrNull(mainTrackIndex(v))?.id }

    // A clip's panels go when the clip does.
    LaunchedEffect(selected?.id) {
        if (selected == null && panel in CLIP_PANELS) panel = null
        if (selected != null && panel == Panel.TextNew) panel = null
    }

    BoxWithConstraints(Modifier.fillMaxSize().background(ConcatColors.page).safeDrawingPadding()) {
        val total = maxHeight
        val imeVisible = WindowInsets.isImeVisible
        val previewHeight: Dp = if (fullscreen) total - Dimens.topBar - Dimens.transport else total * Dimens.PREVIEW_SHARE
        Column(Modifier.fillMaxSize()) {
            TopBar(
                name = view?.name ?: "",
                resolution = view?.let { resolutionLabel(it) } ?: "",
                canExport = view != null && view.clips.isNotEmpty(),
                onClose = { model.close(onClosed) },
                onRename = { renaming = true },
                onExport = { exporting = true },
            )

            // The preview, letterboxed on the page.
            BoxWithConstraints(
                Modifier.fillMaxWidth().height(previewHeight).padding(vertical = 6.dp),
                contentAlignment = Alignment.Center,
            ) {
                if (view != null) {
                    val aspect = if (view.height > 0) view.width.toFloat() / view.height else 9f / 16f
                    val roomW = maxWidth - 24.dp
                    val roomH = maxHeight
                    val (w, h) = if (roomW / roomH > aspect) Pair(roomH * aspect, roomH) else Pair(roomW, roomW / aspect)
                    Box(Modifier.size(w, h).background(Color.Black)) {
                        PreviewSurface(model, Modifier.fillMaxSize())
                        if (ui.previewSlower) SlowerPreviewNote(Modifier.align(Alignment.BottomCenter))
                        PreviewOverlay(
                            view = view,
                            position = ui.position,
                            selected = if (inlineEdit != null) null else ui.selected,
                            mainTrackId = mainTrackId,
                            bounds = ui.bounds,
                            onSelect = {
                                model.select(it)
                                if (it != null) menu = Menu.Root
                            },
                            onCommit = { id, p -> model.setTransform(id, p.scale, p.offsetX, p.offsetY, p.rotation) },
                            onEditText = {
                                model.pause()
                                inlineEdit = it
                            },
                        )
                        inlineEdit?.let { id ->
                            view.clip(id)?.let { clip ->
                                InlineTextEditor(clip, w, h, onDone = { content ->
                                    val style = clip.text
                                    model.saveText(
                                        clip.id, content,
                                        style?.optDouble("fontSize", 0.07) ?: 0.07,
                                        style?.optString("color", "#ffffff") ?: "#ffffff",
                                    )
                                    inlineEdit = null
                                })
                            }
                        }
                    }
                }
            }

            Transport(
                position = ui.position,
                duration = view?.duration ?: 0.0,
                playing = ui.playing,
                canUndo = view?.canUndo == true,
                canRedo = view?.canRedo == true,
                keyframe = selected?.takeIf { it.placeable }?.let { clip ->
                    val at = ((ui.position - clip.start) / clip.duration.coerceAtLeast(1e-6)).coerceIn(0.0, 1.0)
                    clip.keyedAt(at, 0.5 / ((view?.fps ?: 30.0) * clip.duration.coerceAtLeast(1e-6)))
                },
                fullscreen = fullscreen,
                onPlay = model::togglePlay,
                onUndo = model::undo,
                onRedo = model::redo,
                onKeyframe = model::toggleKeyframe,
                onFullscreen = { fullscreen = !fullscreen },
            )

            if (!fullscreen) {
                // Clipped, so a panel sliding up never draws over the toolbar.
                BoxWithConstraints(Modifier.fillMaxWidth().weight(1f).clipToBounds()) {
                    val room = maxHeight
                    if (view != null) {
                        Timeline(
                            view = view,
                            projectPath = path,
                            media = application.concat.media,
                            position = ui.position,
                            selected = ui.selected,
                            onScrub = model::scrubTo,
                            onSelect = {
                                model.select(it)
                                if (it != null) menu = Menu.Root
                            },
                            onMove = model::move,
                            onTrim = model::trim,
                            onAddMedia = ::addMedia,
                            onTransition = { next ->
                                model.select(next)
                                panel = Panel.Transitions
                            },
                            onMuteTrack = model::setTrackSoundMuted,
                            onCover = {
                                model.select(null)
                                panel = Panel.Cover
                            },
                            cover = coverImage,
                        )
                    }
                    PanelSlot(visible = panel != null, modifier = Modifier.align(Alignment.BottomCenter)) {
                        Box(Modifier.fillMaxWidth().heightIn(max = room)) {
                            val close = { panel = null }
                            when (shownPanel) {
                                Panel.Speed -> selected?.let { SpeedPanel(it, model::setSpeed, close) }
                                Panel.Volume -> selected?.let { VolumePanel(it, model::setVolume, model::toggleMute, close) }
                                Panel.Opacity -> selected?.let { OpacityPanel(it, model::setOpacity, close) }
                                Panel.Fade -> selected?.let { FadePanel(it, model::setFades, close) }
                                Panel.Crop -> selected?.let { CropPanel(it, { patch -> model.setLook("Crop", patch) }, close) }
                                Panel.Mask -> selected?.let { MaskPanel(it, { patch -> model.setLook("Mask", patch) }, close) }
                                Panel.Animation -> selected?.let { AnimationPanel(it, { patch -> model.setLook("Animation", patch) }, close) }
                                Panel.TextNew -> TextPanel(null, { c, s, col, wt -> model.saveText(null, c, s, col, wt) }, close)
                                Panel.TextEdit -> selected?.let { clip ->
                                    TextPanel(
                                        clip,
                                        { c, s, col, wt -> model.saveText(clip.id, c, s, col, wt) },
                                        close,
                                        caption = view?.tracks?.firstOrNull { it.id == clip.trackId }?.captions != null,
                                    )
                                }
                                Panel.Effects -> PackagePanel(Shelf.Effects, catalogue, model, onMore = {
                                    catalogueAdjust = false
                                    catalogueShelf = Shelf.Effects
                                }, onClose = close)
                                Panel.Filters -> PackagePanel(Shelf.Filters, catalogue, model, onMore = {
                                    catalogueAdjust = false
                                    catalogueShelf = Shelf.Filters
                                }, onClose = close)
                                Panel.Transitions -> PackagePanel(Shelf.Transitions, catalogue, model, onMore = {
                                    catalogueAdjust = false
                                    catalogueShelf = Shelf.Transitions
                                }, onClose = close)
                                Panel.Aspect -> view?.let { v -> AspectPanel(v, { model.setAspect(it.width, it.height) }, close) }
                                Panel.Background -> BackgroundPanel(model::setBackground, close)
                                Panel.Cover -> CoverPanel(
                                    ui.cover,
                                    named = view?.cover != null,
                                    onFrame = model::setCoverFrame,
                                    onPicture = { pickCover.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
                                    onReset = model::resetCover,
                                    onClose = close,
                                )
                                Panel.Stickers -> StickersPanel(ui.stickers, { card -> model.addSticker(card, stickerName(card)) }, close)
                                Panel.Captions -> CaptionsSheet(captions, model, preferClip = selected?.id, onDismiss = close)
                                Panel.Speech -> SpeechSheet(speech, model, onDismiss = close)
                                Panel.Voiceover -> VoiceoverSheet(voiceover, model, onDismiss = close)
                                null -> Unit
                            }
                        }
                    }
                }

                if (!imeVisible) {
                    val tools = toolsFor(
                        selected = selected,
                        menu = menu,
                        mainTrackId = mainTrackId,
                        panel = panel,
                        actions = ToolActions(
                            openPanel = { panel = if (panel == it) null else it },
                            setMenu = { menu = it },
                            deselect = {
                                model.select(null)
                                panel = null
                            },
                            addMedia = ::addMedia,
                            addOverlay = { pickOverlay.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) },
                            addAudio = { pickAudio.launch(arrayOf("audio/*")) },
                            replace = { sound ->
                                if (sound) pickReplacementSound.launch(arrayOf("audio/*"))
                                else pickReplacement.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                            },
                            reversing = ui.reversing,
                            selectMain = { model.selectAtPlayhead(mainTrackId) },
                            split = {
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                model.split()
                            },
                            catalogue = { shelf, adjust ->
                                catalogueAdjust = adjust
                                catalogueShelf = shelf
                            },
                            model = model,
                        ),
                    )
                    Toolbar(tools, key = "${selected?.kind}-${selected != null}-$menu")
                }
            }
        }

        // What the editor has to say, for a moment, over the bottom of the preview.
        ui.message?.let { message ->
            LaunchedEffect(message) {
                delay(3500)
                model.dismissMessage()
            }
            Box(
                Modifier.align(Alignment.TopCenter).padding(top = Dimens.topBar + previewHeight - 52.dp).padding(horizontal = Spacing.xl)
                    .clip(RoundedCornerShape(Radii.lg)).background(ConcatColors.raised).clickable { model.dismissMessage() }
                    .padding(horizontal = Spacing.lg, vertical = 10.dp),
            ) { Text(message, style = ConcatType.label, color = ConcatColors.fg, textAlign = TextAlign.Center) }
        }
    }

    catalogueShelf?.let { shelf ->
        CatalogueSheet(shelf, catalogue, model, catalogueAdjust, onDismiss = { catalogueShelf = null })
    }
    if (exporting) ExportSheet(model, onDismiss = { exporting = false })
    if (renaming && view != null) {
        RenameDialog(view.name, onDismiss = { renaming = false }) {
            model.rename(it)
            renaming = false
        }
    }
}

/** Where a tool's panel slides up from the toolbar, and back down. */
@Composable
private fun PanelSlot(visible: Boolean, modifier: Modifier, content: @Composable () -> Unit) {
    AnimatedVisibility(
        visible = visible,
        modifier = modifier,
        enter = slideInVertically(Motion.tweenMedium()) { it } + fadeIn(Motion.tweenShort()),
        exit = slideOutVertically(Motion.tweenShort()) { it } + fadeOut(Motion.tweenShort()),
    ) { content() }
}

private fun resolutionLabel(view: ProjectView): String = when (minOf(view.width, view.height)) {
    in 2000..Int.MAX_VALUE -> "4K"
    in 1300..1999 -> "2K"
    in 1000..1299 -> "1080P"
    in 700..999 -> "720P"
    else -> "${minOf(view.width, view.height)}P"
}

@Composable
private fun TopBar(
    name: String,
    resolution: String,
    canExport: Boolean,
    onClose: () -> Unit,
    onRename: () -> Unit,
    onExport: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().height(Dimens.topBar).padding(horizontal = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconAction(ConcatIcons.Close, "Save and close", onClose)
        Box(Modifier.weight(1f).fillMaxHeight().clickable(onClickLabel = "Rename", onClick = onRename), contentAlignment = Alignment.Center) {
            Text(name, style = ConcatType.heading, color = ConcatColors.fg, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(horizontal = Spacing.sm))
        }
        if (resolution.isNotEmpty()) {
            Box(
                Modifier.height(28.dp).clip(RoundedCornerShape(Radii.sm)).border(1.dp, ConcatColors.lineStrong, RoundedCornerShape(Radii.sm))
                    .clickable(onClickLabel = "Export settings", onClick = onExport).padding(horizontal = 8.dp),
                contentAlignment = Alignment.Center,
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(resolution, style = ConcatType.badge.copy(fontSize = 11.sp), color = ConcatColors.fg)
                    ConcatIcon(ConcatIcons.ChevronDown, tint = ConcatColors.fgMuted, size = 14.dp)
                }
            }
        }
        Spacer(Modifier.width(Spacing.sm))
        AccentButton("Export", onExport, enabled = canExport, height = 32.dp, modifier = Modifier.padding(end = Spacing.xs))
    }
}

@Composable
private fun Transport(
    position: Double,
    duration: Double,
    playing: Boolean,
    canUndo: Boolean,
    canRedo: Boolean,
    keyframe: Boolean?,
    fullscreen: Boolean,
    onPlay: () -> Unit,
    onUndo: () -> Unit,
    onRedo: () -> Unit,
    onKeyframe: () -> Unit,
    onFullscreen: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().height(Dimens.transport).padding(horizontal = Spacing.md),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            buildAnnotatedString {
                withStyle(SpanStyle(color = ConcatColors.fg)) { append(shortTimecode(position)) }
                withStyle(SpanStyle(color = ConcatColors.fgDim)) { append(" / " + shortTimecode(duration)) }
            },
            style = ConcatType.timecode,
            modifier = Modifier.weight(1f),
        )
        IconAction(if (playing) ConcatIcons.Pause else ConcatIcons.Play, if (playing) "Pause" else "Play", onPlay, enabled = duration > 0, size = 24.dp, box = 44.dp)
        Row(Modifier.weight(1f), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
            if (keyframe != null) {
                IconAction(
                    if (keyframe) ConcatIcons.KeyframeFilled else ConcatIcons.Keyframe,
                    "Keyframe",
                    onKeyframe,
                    tint = if (keyframe) ConcatColors.accent else ConcatColors.fg,
                    size = 18.dp,
                    box = 36.dp,
                )
            }
            IconAction(ConcatIcons.Undo, "Undo", onUndo, enabled = canUndo, size = 20.dp, box = 36.dp)
            IconAction(ConcatIcons.Redo, "Redo", onRedo, enabled = canRedo, size = 20.dp, box = 36.dp)
            IconAction(ConcatIcons.Maximize, if (fullscreen) "Leave full screen" else "Full screen", onFullscreen, size = 18.dp, box = 36.dp,
                tint = if (fullscreen) ConcatColors.accent else ConcatColors.fg)
        }
    }
}

@Composable
private fun Toolbar(tools: List<Tool>, key: String) {
    Box(Modifier.fillMaxWidth().height(Dimens.toolbar).background(ConcatColors.launchPanel)) {
        Box(Modifier.fillMaxWidth().height(1.dp).background(ConcatColors.line))
        // Keyed by what is selected, not by the tools' states, so a tool
        // lighting up does not swap the row (and lose its scroll).
        Crossfade(targetState = key, animationSpec = Motion.tweenShort(), label = "toolbar") { _ ->
            val shown = tools
            Row(
                Modifier.fillMaxSize().horizontalScroll(rememberScrollState()).padding(horizontal = Spacing.xs, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                for ((i, tool) in shown.withIndex()) {
                    if (i == 0 && tool.label.isEmpty()) {
                        // The way back: a narrow chevron and a rule after it.
                        Box(
                            Modifier.width(44.dp).fillMaxHeight().clip(RoundedCornerShape(Radii.md)).clickable(onClickLabel = "Back", onClick = tool.onClick),
                            contentAlignment = Alignment.Center,
                        ) { ConcatIcon(tool.icon, size = 22.dp, label = "Back") }
                        Box(Modifier.width(1.dp).height(32.dp).background(ConcatColors.lineStrong))
                        Spacer(Modifier.width(Spacing.xs))
                    } else {
                        ToolButton(tool.icon, tool.label, tool.onClick, active = tool.active, enabled = tool.enabled)
                    }
                }
            }
        }
    }
}

private class ToolActions(
    val openPanel: (Panel) -> Unit,
    val setMenu: (Menu) -> Unit,
    val deselect: () -> Unit,
    val addMedia: () -> Unit,
    val addOverlay: () -> Unit,
    val addAudio: () -> Unit,
    /** A file picked to stand in for the selected clip's: a sound file when true. */
    val replace: (Boolean) -> Unit,
    /** The clip a reverse is being written for, and how far it is. */
    val reversing: Pair<String, Float>?,
    val selectMain: () -> Unit,
    val split: () -> Unit,
    val catalogue: (Shelf, Boolean) -> Unit,
    val model: EditorViewModel,
)

/** The toolbar for what is selected (or the menu open when nothing is): only what the engine does. */
private fun toolsFor(selected: Clip?, menu: Menu, mainTrackId: String?, panel: Panel?, actions: ToolActions): List<Tool> {
    val a = actions
    val m = a.model
    fun open(which: Panel) = { a.openPanel(which) }
    if (selected == null) {
        return when (menu) {
            Menu.Root -> listOf(
                Tool(ConcatIcons.Razor, "Edit", onClick = a.selectMain),
                Tool(ConcatIcons.Music, "Audio") { a.setMenu(Menu.Audio) },
                Tool(ConcatIcons.TextMark, "Text") { a.setMenu(Menu.Text) },
                Tool(ConcatIcons.Sticker, "Stickers", active = panel == Panel.Stickers, onClick = open(Panel.Stickers)),
                Tool(ConcatIcons.Image, "Overlay", onClick = a.addOverlay),
                Tool(ConcatIcons.Sparkle, "Effects", active = panel == Panel.Effects) {
                    a.selectMain()
                    a.openPanel(Panel.Effects)
                },
                Tool(ConcatIcons.Filter, "Filters", active = panel == Panel.Filters) {
                    a.selectMain()
                    a.openPanel(Panel.Filters)
                },
                Tool(ConcatIcons.Rows, "Captions", active = panel == Panel.Captions, onClick = open(Panel.Captions)),
                Tool(ConcatIcons.Transition, "Transitions", active = panel == Panel.Transitions, onClick = open(Panel.Transitions)),
                Tool(ConcatIcons.Fit, "Aspect", active = panel == Panel.Aspect, onClick = open(Panel.Aspect)),
                Tool(ConcatIcons.Slot, "Background", active = panel == Panel.Background, onClick = open(Panel.Background)),
                Tool(ConcatIcons.Sliders, "Adjust") {
                    a.selectMain()
                    a.catalogue(Shelf.Effects, true)
                },
                Tool(ConcatIcons.Import, "Media", onClick = a.addMedia),
            )
            Menu.Audio -> listOf(
                Tool(ConcatIcons.ChevronLeft, "") { a.setMenu(Menu.Root) },
                Tool(ConcatIcons.Import, "Sounds", onClick = a.addAudio),
                Tool(ConcatIcons.Mic, "Voiceover", active = panel == Panel.Voiceover, onClick = open(Panel.Voiceover)),
                Tool(ConcatIcons.Waveform, "Text to speech", active = panel == Panel.Speech, onClick = open(Panel.Speech)),
                Tool(ConcatIcons.Rows, "Captions", active = panel == Panel.Captions, onClick = open(Panel.Captions)),
            )
            Menu.Text -> listOf(
                Tool(ConcatIcons.ChevronLeft, "") { a.setMenu(Menu.Root) },
                Tool(ConcatIcons.Plus, "Add text", active = panel == Panel.TextNew, onClick = open(Panel.TextNew)),
                Tool(ConcatIcons.Rows, "Auto captions", active = panel == Panel.Captions, onClick = open(Panel.Captions)),
                Tool(ConcatIcons.Waveform, "Text to speech", active = panel == Panel.Speech, onClick = open(Panel.Speech)),
            )
        }
    }
    val back = Tool(ConcatIcons.ChevronLeft, "", onClick = a.deselect)
    if (selected.kind == ClipKind.Transition) {
        return listOf(
            back,
            Tool(ConcatIcons.Transition, "Change", active = panel == Panel.Transitions, onClick = open(Panel.Transitions)),
            Tool(ConcatIcons.Sliders, "Adjust") { a.catalogue(Shelf.Transitions, true) },
            Tool(ConcatIcons.Copy, "Duplicate", onClick = m::duplicate),
            Tool(ConcatIcons.Trash, "Delete", onClick = m::delete),
        )
    }
    val picture = selected.kind == ClipKind.Video || selected.kind == ClipKind.Image
    val sound = selected.kind == ClipKind.Video || selected.kind == ClipKind.Audio
    val moved = selected.scale != 1.0 || selected.offsetX != 0.0 || selected.offsetY != 0.0 || selected.rotation != 0.0
    return buildList {
        add(back)
        if (selected.kind == ClipKind.Text) add(Tool(ConcatIcons.TextMark, "Edit", active = panel == Panel.TextEdit, onClick = open(Panel.TextEdit)))
        add(Tool(ConcatIcons.Split, "Split", onClick = a.split))
        if (selected.kind != ClipKind.Text) add(Tool(ConcatIcons.SpeedConstant, "Speed", active = panel == Panel.Speed, onClick = open(Panel.Speed)))
        if (sound) add(Tool(ConcatIcons.Volume, "Volume", active = panel == Panel.Volume, onClick = open(Panel.Volume)))
        if (sound) add(Tool(ConcatIcons.Curves, "Fade", active = panel == Panel.Fade, onClick = open(Panel.Fade)))
        add(Tool(ConcatIcons.Trash, "Delete", onClick = m::delete))
        add(Tool(ConcatIcons.Copy, "Duplicate", onClick = m::duplicate))
        add(Tool(ConcatIcons.Sparkle, "Effects", active = panel == Panel.Effects, onClick = open(Panel.Effects)))
        if (selected.kind != ClipKind.Audio) add(Tool(ConcatIcons.Filter, "Filters", active = panel == Panel.Filters, onClick = open(Panel.Filters)))
        add(Tool(ConcatIcons.Sliders, "Adjust") { a.catalogue(Shelf.Effects, true) })
        if (selected.kind != ClipKind.Audio) add(Tool(ConcatIcons.Eye, "Opacity", active = panel == Panel.Opacity, onClick = open(Panel.Opacity)))
        if (picture) add(Tool(ConcatIcons.FlipH, "Flip", active = selected.flipH, onClick = m::flip))
        if (selected.placeable) add(Tool(ConcatIcons.Wand, "Animation", active = panel == Panel.Animation, onClick = open(Panel.Animation)))
        if (picture) add(Tool(ConcatIcons.Frame, "Crop", active = panel == Panel.Crop, onClick = open(Panel.Crop)))
        if (picture) add(Tool(ConcatIcons.MaskCircle, "Mask", active = panel == Panel.Mask, onClick = open(Panel.Mask)))
        if (picture) add(Tool(ConcatIcons.Pause, "Freeze", onClick = m::freeze))
        if (selected.kind == ClipKind.Video || selected.kind == ClipKind.Audio) {
            val running = a.reversing
            when {
                running?.first == selected.id ->
                    add(Tool(ConcatIcons.Reverse, "Reversing ${(running.second * 100).toInt()}%", active = true, onClick = m::cancelReverse))
                else -> add(Tool(ConcatIcons.Reverse, "Reverse", enabled = running == null, onClick = m::reverse))
            }
        }
        if (picture || selected.kind == ClipKind.Audio) add(Tool(ConcatIcons.Import, "Replace") { a.replace(selected.kind == ClipKind.Audio) })
        if (selected.kind == ClipKind.Video) add(Tool(if (selected.muted) ConcatIcons.VolumeOff else ConcatIcons.Volume, if (selected.muted) "Unmute" else "Mute", onClick = m::toggleMute))
        if (selected.placeable) add(Tool(ConcatIcons.Reset, "Reset", enabled = moved, onClick = m::resetTransform))
        if (selected.kind.hasSound) add(Tool(ConcatIcons.Rows, "Captions", active = panel == Panel.Captions, onClick = open(Panel.Captions)))
        if (picture && selected.trackId == mainTrackId) add(Tool(ConcatIcons.Transition, "Transition", active = panel == Panel.Transitions, onClick = open(Panel.Transitions)))
    }
}

/** A title's words edited where they sit: the keyboard up, the words over a dimmed frame. */
@Composable
private fun InlineTextEditor(clip: Clip, frameWidth: Dp, frameHeight: Dp, onDone: (String) -> Unit) {
    var content by remember(clip.id) { mutableStateOf(clip.text?.optString("content") ?: clip.name) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(clip.id) { focus.requestFocus() }
    val colour = colourOf(clip.text?.optString("color", "#ffffff") ?: "#ffffff")
    val size = ((clip.text?.optDouble("fontSize", 0.07) ?: 0.07) * frameHeight.value * clip.scale).coerceIn(14.0, 40.0)
    Box(
        Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.72f))
            .clickable(remember { MutableInteractionSource() }, indication = null) { onDone(content) },
    ) {
        BasicTextField(
            value = content,
            onValueChange = { content = it },
            textStyle = ConcatType.title.copy(color = colour, fontSize = size.sp, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center),
            cursorBrush = SolidColor(ConcatColors.accent),
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
            // Where the title sits on the frame, upright, so the words are edited in place.
            modifier = Modifier.align(Alignment.Center)
                .offset(x = frameWidth * clip.offsetX.toFloat(), y = frameHeight * clip.offsetY.toFloat())
                .fillMaxWidth().padding(horizontal = Spacing.lg).focusRequester(focus),
        )
        Box(
            Modifier.align(Alignment.TopEnd).padding(Spacing.sm).size(34.dp).clip(CircleShape).background(ConcatColors.accent)
                .clickable(onClickLabel = "Done") { onDone(content) },
            contentAlignment = Alignment.Center,
        ) { ConcatIcon(ConcatIcons.Check, tint = ConcatColors.onAccent, size = 20.dp, label = "Done") }
    }
}

@Composable
private fun RenameDialog(current: String, onDismiss: () -> Unit, onRename: (String) -> Unit) {
    var name by remember { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = ConcatColors.launchRaised,
        title = { Text("Rename project", style = ConcatType.title) },
        text = { OutlinedTextField(value = name, onValueChange = { name = it }, singleLine = true) },
        confirmButton = { TextButton(onClick = { onRename(name) }, enabled = name.isNotBlank()) { Text("Rename") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * Said over the preview once it draws the slower way: this phone's graphics
 * failed under it, so playback may stutter. Exports are not affected.
 */
@Composable
private fun SlowerPreviewNote(modifier: Modifier) {
    Text(
        "Preview runs in a slower mode on this phone. Exports are not affected.",
        modifier = modifier
            .padding(6.dp)
            .background(Color.Black.copy(alpha = 0.6f), RoundedCornerShape(6.dp))
            .padding(horizontal = 8.dp, vertical = 4.dp),
        style = ConcatType.caption,
        color = ConcatColors.fgMuted,
    )
}

/** The engine's picture: a SurfaceView it draws on, attached and detached with the surface's life. */
@Composable
private fun PreviewSurface(model: EditorViewModel, modifier: Modifier) {
    AndroidView(
        modifier = modifier,
        factory = { context ->
            SurfaceView(context).apply {
                holder.addCallback(object : SurfaceHolder.Callback {
                    override fun surfaceCreated(holder: SurfaceHolder) {}
                    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                        model.surfaceChanged(holder.surface, width, height)
                    }
                    override fun surfaceDestroyed(holder: SurfaceHolder) {
                        model.surfaceDestroyed()
                    }
                })
            }
        },
    )
}
