// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import android.app.Application
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import app.concat.editor.spike.api.ClipKind
import app.concat.editor.spike.ui.Palette
import app.concat.editor.spike.ui.timecode

private enum class Sheet { None, Text, Speed, Volume, Export, Captions, Speech, Voiceover, Effects, Filters, Transitions }

@Composable
fun EditorScreen(path: String, session: Int, onClosed: () -> Unit) {
    val application = LocalContext.current.applicationContext as Application
    val model: EditorViewModel = viewModel(
        key = "$path#$session",
        factory = viewModelFactory { initializer { EditorViewModel(application, path) } },
    )
    // The sheets' own models, one each per open project: their jobs outlive the sheet.
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
    var sheet by remember { mutableStateOf(Sheet.None) }
    var editingText by remember { mutableStateOf<String?>(null) }
    var adjustFirst by remember { mutableStateOf(false) }
    fun shelf(which: Sheet, adjust: Boolean = false) {
        adjustFirst = adjust
        sheet = which
    }

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
    BackHandler { model.close(onClosed) }

    val pickVisual = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(50)) { uris ->
        model.import(uris, audio = false)
    }
    val pickAudio = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        model.import(uris, audio = true)
    }

    val view = ui.view
    val selected = view?.clip(ui.selected)
    val fps = view?.fps ?: 30.0

    Column(Modifier.fillMaxSize().background(Palette.background).safeDrawingPadding()) {
        // Top bar.
        Row(Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = { model.close(onClosed) }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Save and close", tint = Palette.text)
            }
            Text(
                view?.name ?: "",
                color = Palette.text,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Button(
                onClick = { sheet = Sheet.Export },
                enabled = view != null && view.clips.isNotEmpty(),
                shape = RoundedCornerShape(8.dp),
                modifier = Modifier.padding(end = 8.dp),
            ) { Text("Export") }
        }
        if (ui.busy) LinearProgressIndicator(Modifier.fillMaxWidth().height(2.dp)) else Spacer(Modifier.height(2.dp))

        // Preview.
        Box(Modifier.fillMaxWidth().weight(1f).background(Color.Black), contentAlignment = Alignment.Center) {
            if (view != null) {
                val aspect = if (view.height > 0) view.width.toFloat() / view.height else 9f / 16f
                PreviewSurface(model, Modifier.aspectRatio(aspect, matchHeightConstraintsFirst = aspect < 1f))
            }
        }

        // Transport.
        Row(
            Modifier.fillMaxWidth().height(44.dp).padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "${timecode(ui.position, fps)} / ${timecode(view?.duration ?: 0.0, fps)}",
                color = Palette.text,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = model::togglePlay, enabled = (view?.duration ?: 0.0) > 0) {
                if (ui.playing) PauseGlyph() else Icon(Icons.Filled.PlayArrow, contentDescription = "Play", tint = Palette.text)
            }
            Row(Modifier.weight(1f), horizontalArrangement = Arrangement.End) {
                GlyphButton("↶", "Undo", enabled = view?.canUndo == true, onClick = model::undo)
                GlyphButton("↷", "Redo", enabled = view?.canRedo == true, onClick = model::redo)
            }
        }

        // Timeline.
        Box(Modifier.fillMaxWidth().height(200.dp)) {
            if (view != null) {
                Timeline(
                    view,
                    ui.position,
                    ui.selected,
                    onScrub = model::scrubTo,
                    onSelect = model::select,
                    onMove = model::move,
                    onTrim = model::trim,
                )
            }
        }

        // Tools for what is selected.
        Row(
            Modifier.fillMaxWidth().height(72.dp).background(Palette.surface).horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (selected == null) {
                Tool("Media", icon = Icons.Filled.Add) {
                    pickVisual.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                }
                Tool("Text", glyph = "Aa") {
                    editingText = null
                    sheet = Sheet.Text
                }
                Tool("Audio", glyph = "♪") { pickAudio.launch(arrayOf("audio/*")) }
                Tool("Captions", glyph = "CC") { sheet = Sheet.Captions }
                Tool("Read aloud", glyph = "❝") { sheet = Sheet.Speech }
                Tool("Voiceover", glyph = "◉") { sheet = Sheet.Voiceover }
                Tool("Transition", glyph = "⋈") { shelf(Sheet.Transitions) }
            } else if (selected.kind == ClipKind.Transition) {
                Tool("Done", glyph = "✓") { model.select(null) }
                Tool("Adjust", glyph = "≡") { shelf(Sheet.Transitions, adjust = true) }
                Tool("Change", glyph = "⋈") { shelf(Sheet.Transitions) }
                Tool("Duplicate", glyph = "⧉") { model.duplicate() }
                Tool("Delete", icon = Icons.Filled.Delete) { model.delete() }
            } else {
                Tool("Done", glyph = "✓") { model.select(null) }
                if (selected.kind == ClipKind.Text) {
                    Tool("Edit", icon = Icons.Filled.Edit) {
                        editingText = selected.id
                        sheet = Sheet.Text
                    }
                }
                Tool("Split", glyph = "✂") { model.split() }
                Tool("Delete", icon = Icons.Filled.Delete) { model.delete() }
                Tool("Duplicate", glyph = "⧉") { model.duplicate() }
                if (selected.kind != ClipKind.Text) {
                    Tool("Speed", glyph = "1x") { sheet = Sheet.Speed }
                }
                if (selected.kind == ClipKind.Video || selected.kind == ClipKind.Audio) {
                    Tool("Volume", glyph = "◁)") { sheet = Sheet.Volume }
                }
                if (selected.kind.hasSound) {
                    Tool("Captions", glyph = "CC") { sheet = Sheet.Captions }
                }
                Tool("Effects", glyph = "✦") { shelf(Sheet.Effects) }
                if (selected.kind != ClipKind.Audio) {
                    Tool("Filters", glyph = "◐") { shelf(Sheet.Filters) }
                }
                Tool("Transition", glyph = "⋈") { shelf(Sheet.Transitions) }
            }
        }
    }

    when (sheet) {
        Sheet.Text -> TextSheet(
            existing = view?.clip(editingText),
            onDismiss = { sheet = Sheet.None },
            onSave = { content, size, color ->
                model.saveText(editingText, content, size, color)
                sheet = Sheet.None
            },
        )
        Sheet.Speed -> selected?.let { clip ->
            SliderSheet("Speed", clip.speed, 0.25f..4f, { "%.2fx".format(it) }, onDismiss = { sheet = Sheet.None }) {
                model.setSpeed(it)
                sheet = Sheet.None
            }
        }
        Sheet.Volume -> selected?.let { clip ->
            SliderSheet("Volume", clip.volume, 0f..2f, { "${(it * 100).toInt()}%" }, onDismiss = { sheet = Sheet.None }) {
                model.setVolume(it)
                sheet = Sheet.None
            }
        }
        Sheet.Export -> ExportSheet(model, onDismiss = { sheet = Sheet.None })
        Sheet.Captions -> CaptionsSheet(captions, model, preferClip = selected?.id, onDismiss = { sheet = Sheet.None })
        Sheet.Speech -> SpeechSheet(speech, model, onDismiss = { sheet = Sheet.None })
        Sheet.Voiceover -> VoiceoverSheet(voiceover, model, onDismiss = { sheet = Sheet.None })
        Sheet.Effects -> CatalogueSheet(Shelf.Effects, catalogue, model, adjustFirst, onDismiss = { sheet = Sheet.None })
        Sheet.Filters -> CatalogueSheet(Shelf.Filters, catalogue, model, adjustFirst, onDismiss = { sheet = Sheet.None })
        Sheet.Transitions -> CatalogueSheet(Shelf.Transitions, catalogue, model, adjustFirst, onDismiss = { sheet = Sheet.None })
        Sheet.None -> Unit
    }

    ui.message?.let { message ->
        AlertDialog(
            onDismissRequest = model::dismissMessage,
            confirmButton = { TextButton(onClick = model::dismissMessage) { Text("OK") } },
            text = { Text(message) },
        )
    }
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

@Composable
private fun Tool(label: String, icon: ImageVector? = null, glyph: String? = null, onClick: () -> Unit) {
    Column(
        Modifier.width(68.dp).fillMaxHeight().clip(RoundedCornerShape(8.dp)).clickable(onClick = onClick).padding(vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) {
            if (icon != null) Icon(icon, contentDescription = null, tint = Palette.text)
            else Text(glyph ?: "", color = Palette.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
        }
        Spacer(Modifier.height(4.dp))
        Text(label, color = Palette.muted, fontSize = 11.sp)
    }
}

@Composable
private fun GlyphButton(glyph: String, label: String, enabled: Boolean, onClick: () -> Unit) {
    IconButton(onClick = onClick, enabled = enabled, modifier = Modifier.semantics { contentDescription = label }) {
        Text(glyph, fontSize = 22.sp, color = if (enabled) Palette.text else Palette.muted.copy(alpha = 0.5f))
    }
}

@Composable
private fun PauseGlyph() {
    Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
        Box(Modifier.width(5.dp).height(18.dp).background(Palette.text))
        Box(Modifier.width(5.dp).height(18.dp).background(Palette.text))
    }
}
