// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import app.concat.editor.ui.theme.ConcatColors
import app.concat.editor.ui.theme.ConcatType
import app.concat.editor.ui.theme.Spacing

/**
 * What a release build shows when its engine will not start: the reason,
 * and nothing to edit with, since without the engine nothing would be kept.
 */
@Composable
fun EngineFailedScreen(failure: Throwable) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(ConcatColors.page)
            .safeDrawingPadding()
            .padding(Spacing.xl),
        verticalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Text("Concat can't start", style = ConcatType.display, color = ConcatColors.fg)
        Text(
            failure.message ?: failure.javaClass.simpleName,
            style = ConcatType.body,
            color = ConcatColors.fgMuted,
        )
        Text(
            "Your projects are untouched. Reinstalling Concat usually fixes this; if it doesn't, " +
                "please report it at github.com/jub0t/Concat/issues with your phone's model.",
            style = ConcatType.body,
            color = ConcatColors.fgMuted,
        )
    }
}
