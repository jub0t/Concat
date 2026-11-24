// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui

import androidx.compose.ui.graphics.Color
import app.concat.editor.api.ClipKind
import app.concat.editor.ui.theme.ConcatColors
import app.concat.editor.ui.theme.KindColors

/**
 * The older screens' names for the theme's tokens (ui.theme): the sheets
 * and settings read these, so they follow the design without each being
 * rewritten.
 */
object Palette {
    val background get() = ConcatColors.page
    val surface get() = ConcatColors.launchPanel
    val raised get() = ConcatColors.raised
    val accent get() = ConcatColors.accent
    val text get() = ConcatColors.fg
    val muted get() = ConcatColors.fgMuted

    fun clip(kind: ClipKind): Color = KindColors.of(kind).color
}

/** `mm:ss.ff` at the project's rate. */
fun timecode(seconds: Double, fps: Double): String {
    val safe = seconds.coerceAtLeast(0.0)
    val whole = safe.toLong()
    val frame = ((safe - whole) * fps.coerceAtLeast(1.0)).toInt()
    return "%02d:%02d.%02d".format(whole / 60, whole % 60, frame)
}

/** `mm:ss`, as the transport row shows it. */
fun shortTimecode(seconds: Double): String {
    val whole = seconds.coerceAtLeast(0.0).toLong()
    return "%02d:%02d".format(whole / 60, whole % 60)
}

/** "3.2s", as a clip's duration badge reads. */
fun secondsLabel(seconds: Double): String =
    if (seconds >= 60) "%d:%04.1f".format((seconds / 60).toInt(), seconds % 60) else "%.1fs".format(seconds)
