// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilterChip
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.concat.editor.spike.api.ModelInfo
import app.concat.editor.spike.models.ModelDownload
import app.concat.editor.spike.models.sizeLabel
import app.concat.editor.spike.ui.Palette

@Composable
fun SheetTitle(text: String) = Text(text, fontWeight = FontWeight.SemiBold, fontSize = 16.sp)

@Composable
fun SectionLabel(text: String) = Text(text, color = Palette.muted, fontSize = 12.sp)

/** One choice of a few, as chips in a row that scrolls when it must. */
@Composable
fun <T> ChoiceChips(options: List<T>, selected: T, label: (T) -> String, onSelect: (T) -> Unit) {
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (option in options) {
            FilterChip(selected = option == selected, onClick = { onSelect(option) }, label = { Text(label(option)) })
        }
    }
}

/**
 * The models of one kind, one row each: picked when downloaded, a
 * Download button (and its progress) when not.
 */
@Composable
fun ModelPicker(
    models: List<ModelInfo>,
    selected: String?,
    downloads: Map<String, ModelDownload>,
    onSelect: (ModelInfo) -> Unit,
    onDownload: (ModelInfo) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (models.isEmpty()) SectionLabel("No models on offer.")
        for (model in models) {
            val picked = model.id == selected
            val downloading = downloads[model.id]
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp))
                    .background(if (picked) Palette.raised else Palette.surface)
                    .border(if (picked) 1.5.dp else 0.dp, if (picked) Palette.accent else Palette.surface, RoundedCornerShape(10.dp))
                    .clickable(enabled = model.downloaded) { onSelect(model) }
                    .padding(horizontal = 12.dp, vertical = 8.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(model.label, color = Palette.text, fontSize = 14.sp, fontWeight = FontWeight.Medium)
                        Text(
                            listOfNotNull(model.blurb.ifBlank { null }, sizeLabel(model.sizeBytes), "English only".takeIf { model.englishOnly })
                                .joinToString(" · "),
                            color = Palette.muted,
                            fontSize = 11.sp,
                        )
                    }
                    Spacer(Modifier.width(8.dp))
                    when {
                        downloading != null -> Text(
                            if (downloading.unpacking) "Unpacking" else "${(downloading.fraction * 100).toInt()}%",
                            color = Palette.accent,
                            fontSize = 12.sp,
                        )
                        !model.downloaded -> OutlinedButton(onClick = { onDownload(model) }) { Text("Download") }
                        picked -> app.concat.editor.spike.ui.components.ConcatIcon(app.concat.editor.spike.ui.icons.ConcatIcons.Check, tint = Palette.accent, size = 20.dp)
                    }
                }
                if (downloading != null) {
                    Spacer(Modifier.height(6.dp))
                    LinearProgressIndicator(progress = { downloading.fraction }, modifier = Modifier.fillMaxWidth())
                }
            }
        }
    }
}

/** A job's progress: what it is doing and how far. */
@Composable
fun JobProgress(stage: String, fraction: Float) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (fraction > 0f) {
            LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
        } else {
            LinearProgressIndicator(Modifier.fillMaxWidth())
        }
        SectionLabel(if (fraction > 0f) "${stage.replaceFirstChar { it.uppercase() }} ${(fraction * 100).toInt()}%" else stage.replaceFirstChar { it.uppercase() })
    }
}
