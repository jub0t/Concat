// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import app.concat.editor.spike.api.ClipKind

/** The editor's palette: near-black, with one bright accent, as phone editors look. */
object Palette {
    val background = Color(0xFF0B0B0C)
    val surface = Color(0xFF161618)
    val raised = Color(0xFF222226)
    val accent = Color(0xFF25E2CF)
    val text = Color(0xFFF2F2F2)
    val muted = Color(0xFF8E8E93)

    fun clip(kind: ClipKind): Color = when (kind) {
        ClipKind.Video -> Color(0xFF2F6FDE)
        ClipKind.Image -> Color(0xFF2A9D8F)
        ClipKind.Audio -> Color(0xFF3FA34D)
        ClipKind.Text -> Color(0xFFB0558F)
        ClipKind.Layer -> Color(0xFF8E6BD6)
        ClipKind.SoundLayer -> Color(0xFF6B8E3F)
        ClipKind.Transition -> Color(0xFFD68A2F)
        ClipKind.Other -> Color(0xFF5A5A60)
    }
}

@Composable
fun ConcatTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Palette.accent,
            onPrimary = Color.Black,
            secondary = Palette.accent,
            background = Palette.background,
            onBackground = Palette.text,
            surface = Palette.surface,
            onSurface = Palette.text,
            surfaceVariant = Palette.raised,
            onSurfaceVariant = Palette.muted,
            surfaceContainer = Palette.surface,
            surfaceContainerHigh = Palette.raised,
            surfaceContainerLow = Palette.surface,
        ),
        content = content,
    )
}

/** `mm:ss.ff` at the project's rate. */
fun timecode(seconds: Double, fps: Double): String {
    val safe = seconds.coerceAtLeast(0.0)
    val whole = safe.toLong()
    val frame = ((safe - whole) * fps.coerceAtLeast(1.0)).toInt()
    return "%02d:%02d.%02d".format(whole / 60, whole % 60, frame)
}
