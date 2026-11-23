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
