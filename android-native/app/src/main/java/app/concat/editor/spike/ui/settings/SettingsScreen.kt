// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.settings

import android.app.Application
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import app.concat.editor.spike.ui.icons.ConcatIcons
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import app.concat.editor.spike.BuildConfig
import app.concat.editor.spike.api.ModelInfo
import app.concat.editor.spike.api.VersionInfo
import app.concat.editor.spike.concat
import app.concat.editor.spike.models.sizeLabel
import app.concat.editor.spike.ui.Palette
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

class SettingsViewModel(application: Application) : AndroidViewModel(application) {
    private val app = application.concat
    val models = app.models
    val fake get() = app.api.engine.isFake
    private val _version = MutableStateFlow<VersionInfo?>(null)
    val version: StateFlow<VersionInfo?> = _version

    fun load() {
        models.refresh()
        viewModelScope.launch {
            _version.value = try {
                app.api.version()
            } catch (_: Exception) {
                null
            }
        }
    }
}

/**
 * Settings: the speech models (download, delete) and what this build is.
 * There is no log to share: the API offers none, so the engine's lines go
 * to logcat only.
 */
@Composable
fun SettingsScreen(onBack: () -> Unit, model: SettingsViewModel = viewModel()) {
    val models by model.models.state.collectAsStateWithLifecycle()
    val version by model.version.collectAsStateWithLifecycle()
    var deleting by remember { mutableStateOf<ModelInfo?>(null) }
    LaunchedEffect(Unit) { model.load() }
    BackHandler(onBack = onBack)

    Column(Modifier.fillMaxSize().background(Palette.background).safeDrawingPadding()) {
        Row(Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) {
                Icon(ConcatIcons.ChevronLeft, contentDescription = "Back", tint = Palette.text)
            }
            Text("Settings", color = Palette.text, fontWeight = FontWeight.SemiBold, fontSize = 18.sp)
        }
        LazyColumn(
            Modifier.fillMaxSize().padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            item { Heading("Captions models") }
            items(models.info.transcribers, key = { "t-${it.id}" }) { info ->
                ModelRow(info, models.downloads[info.id]?.fraction, onDownload = { model.models.download(info) }, onDelete = { deleting = info })
            }
            item { Heading("Read-aloud models") }
            items(models.info.speech, key = { "s-${it.id}" }) { info ->
                ModelRow(info, models.downloads[info.id]?.fraction, onDownload = { model.models.download(info) }, onDelete = { deleting = info })
            }
            if (!models.loaded) item { Text("Loading...", color = Palette.muted, fontSize = 12.sp) }
            item { Heading("About") }
            item {
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Palette.surface).padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    About("App", "${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})")
                    About("Engine", version?.concat ?: "...")
                    About("API", version?.apiVersion ?: "...")
                    About("Runs on", if (model.fake) "In-memory stand-in" else "Native engine")
                    if (!version?.capabilities.isNullOrEmpty()) About("Serves", version?.capabilities.orEmpty().joinToString(", "))
                }
            }
            item { Spacer(Modifier.height(24.dp)) }
        }
    }

    deleting?.let { info ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            confirmButton = {
                TextButton(onClick = {
                    model.models.delete(info)
                    deleting = null
                }) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Keep") } },
            text = { Text("Delete ${info.label}? It frees ${sizeLabel(info.sizeBytes)}; it can be downloaded again.") },
        )
    }
    models.error?.let { message ->
        AlertDialog(
            onDismissRequest = model.models::dismissError,
            confirmButton = { TextButton(onClick = model.models::dismissError) { Text("OK") } },
            text = { Text(message) },
        )
    }
}

@Composable
private fun Heading(text: String) =
    Text(text, color = Palette.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 16.dp, bottom = 4.dp))

@Composable
private fun About(label: String, value: String) {
    Row {
        Text(label, color = Palette.muted, fontSize = 13.sp, modifier = Modifier.width(88.dp))
        Text(value, color = Palette.text, fontSize = 13.sp)
    }
}

@Composable
private fun ModelRow(info: ModelInfo, progress: Float?, onDownload: () -> Unit, onDelete: () -> Unit) {
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Palette.surface).padding(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(info.label, color = Palette.text, fontWeight = FontWeight.Medium)
                Text(
                    listOfNotNull(info.blurb.ifBlank { null }, sizeLabel(info.sizeBytes), "English only".takeIf { info.englishOnly }).joinToString(" · "),
                    color = Palette.muted,
                    fontSize = 11.sp,
                )
            }
            when {
                progress != null -> Text("${(progress * 100).toInt()}%", color = Palette.accent, fontSize = 12.sp)
                info.downloaded && info.sizeBytes > 0 -> OutlinedButton(onClick = onDelete) { Text("Delete") }
                info.downloaded -> Text("Ready", color = Palette.muted, fontSize = 12.sp)
                else -> OutlinedButton(onClick = onDownload) { Text("Download") }
            }
        }
        if (progress != null) {
            Spacer(Modifier.height(8.dp))
            LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
        }
    }
}
