// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.concat.editor.spike.api.Clip
import app.concat.editor.spike.export.EXPORT_PRESETS
import app.concat.editor.spike.export.ExportState
import app.concat.editor.spike.ui.Palette

private val TEXT_COLOURS = listOf("#ffffff", "#000000", "#ffd60a", "#ff375f", "#30d158", "#25e2cf", "#0a84ff", "#bf5af2")

private fun colourOf(hex: String): Color = try {
    Color(android.graphics.Color.parseColor(hex))
} catch (_: Exception) {
    Color.White
}

/** A title's words, size and colour: a new one, or [existing] restyled. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TextSheet(existing: Clip?, onDismiss: () -> Unit, onSave: (String, Double, String) -> Unit) {
    val style = existing?.text
    var content by remember { mutableStateOf(style?.optString("content") ?: "") }
    // The document's size is a fraction of the frame's height.
    var size by remember { mutableFloatStateOf((style?.optDouble("fontSize", 0.09) ?: 0.09).toFloat()) }
    var colour by remember { mutableStateOf(style?.optString("color", "#ffffff") ?: "#ffffff") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(16.dp).imePadding(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(if (existing == null) "Add text" else "Edit text", fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
            OutlinedTextField(
                value = content,
                onValueChange = { content = it },
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                placeholder = { Text("Enter text") },
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                minLines = 1,
                maxLines = 4,
            )
            Text("Size ${(size * 100).toInt()}", color = Palette.muted, fontSize = 12.sp)
            Slider(value = size, onValueChange = { size = it }, valueRange = 0.03f..0.25f)
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                for (hex in TEXT_COLOURS) {
                    Box(
                        Modifier.size(30.dp).background(colourOf(hex), CircleShape)
                            .border(if (hex == colour) 3.dp else 1.dp, if (hex == colour) Palette.accent else Palette.muted, CircleShape)
                            .clickable { colour = hex },
                    )
                }
            }
            Button(
                onClick = { onSave(content.trim(), size.toDouble(), colour) },
                enabled = content.isNotBlank(),
                modifier = Modifier.fillMaxWidth(),
            ) { Text(if (existing == null) "Add" else "Apply") }
            Spacer(Modifier.navigationBarsPadding())
        }
    }
}

/** One number, as speed and volume are set. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SliderSheet(
    title: String,
    initial: Double,
    range: ClosedFloatingPointRange<Float>,
    describe: (Float) -> String,
    onDismiss: () -> Unit,
    onApply: (Double) -> Unit,
) {
    var value by remember { mutableFloatStateOf(initial.toFloat().coerceIn(range)) }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(title, fontWeight = FontWeight.SemiBold, fontSize = 16.sp, modifier = Modifier.weight(1f))
                Text(describe(value), color = Palette.accent)
            }
            Slider(value = value, onValueChange = { value = it }, valueRange = range)
            Button(onClick = { onApply(value.toDouble()) }, modifier = Modifier.fillMaxWidth()) { Text("Apply") }
            Spacer(Modifier.navigationBarsPadding())
        }
    }
}

/** Presets, then progress and Cancel, then Share: the export from start to end. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ExportSheet(model: EditorViewModel, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val state by model.exports.state.collectAsStateWithLifecycle()
    var preset by remember { mutableStateOf(EXPORT_PRESETS[1]) }
    // The progress notification needs the permission from API 33; the
    // export runs either way.
    val askNotifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        model.export(preset)
    }
    fun start() {
        val needs = Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        if (needs) askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS) else model.export(preset)
    }
    val close = {
        model.exports.dismiss()
        onDismiss()
    }

    ModalBottomSheet(onDismissRequest = { if (state !is ExportState.Running) close() else onDismiss() }) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            when (val now = state) {
                ExportState.Idle -> {
                    Text("Export", fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (option in EXPORT_PRESETS) {
                            FilterChip(selected = option == preset, onClick = { preset = option }, label = { Text(option.label) })
                        }
                    }
                    Text("H.264, quality ${preset.crf}, ${preset.preset}", color = Palette.muted, fontSize = 12.sp)
                    Button(onClick = ::start, modifier = Modifier.fillMaxWidth()) { Text("Export") }
                }
                is ExportState.Running -> {
                    Text("Exporting ${now.title}", fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                    LinearProgressIndicator(progress = { now.fraction }, modifier = Modifier.fillMaxWidth())
                    Text("${(now.fraction * 100).toInt()}% - ${now.stage}", color = Palette.muted, fontSize = 12.sp)
                    Text("It carries on if you leave the app.", color = Palette.muted, fontSize = 12.sp)
                    OutlinedButton(onClick = model.exports::cancel, modifier = Modifier.fillMaxWidth()) { Text("Cancel") }
                }
                is ExportState.Done -> {
                    Text("Exported", fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                    Text(now.note ?: "Saved to Movies/Concat", color = Palette.muted, fontSize = 12.sp)
                    Button(
                        onClick = { context.startActivity(model.exports.shareIntent(now)) },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text("Share") }
                    OutlinedButton(onClick = close, modifier = Modifier.fillMaxWidth()) { Text("Done") }
                }
                is ExportState.Failed -> {
                    Text(if (now.cancelled) "Export cancelled" else "Export failed", fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                    if (!now.cancelled) Text(now.message, color = Palette.muted, fontSize = 12.sp)
                    Button(onClick = { model.exports.dismiss() }, modifier = Modifier.fillMaxWidth()) { Text("Back to presets") }
                }
            }
            Spacer(Modifier.height(8.dp).navigationBarsPadding())
        }
    }
}
