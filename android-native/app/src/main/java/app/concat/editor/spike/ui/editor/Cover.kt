// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.editor

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import app.concat.editor.spike.ui.components.AccentButton
import app.concat.editor.spike.ui.components.GhostButton
import app.concat.editor.spike.ui.components.PanelLabel
import app.concat.editor.spike.ui.components.ToolPanel
import app.concat.editor.spike.ui.icons.ConcatIcons
import app.concat.editor.spike.ui.theme.ConcatColors
import app.concat.editor.spike.ui.theme.Radii
import app.concat.editor.spike.ui.theme.Spacing

/**
 * The cover (`setCover`): the frame under the playhead, a picture from the
 * phone, or back to a frame early in the edit. What the projects screen's
 * card and the timeline's cover square show.
 */
@Composable
fun CoverPanel(
    cover: Bitmap?,
    named: Boolean,
    onFrame: () -> Unit,
    onPicture: () -> Unit,
    onReset: () -> Unit,
    onClose: () -> Unit,
) {
    ToolPanel("Cover", onClose = onClose, onConfirm = onClose) {
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.md), verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier.size(72.dp).clip(RoundedCornerShape(Radii.sm)).background(ConcatColors.raised),
                contentAlignment = Alignment.Center,
            ) {
                val image = remember(cover) { cover?.asImageBitmap() }
                if (image != null) {
                    Image(image, contentDescription = "The cover", contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                }
            }
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                PanelLabel(if (named) "Shown on the project's card." else "A frame early in the edit, until you choose one.")
                Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                    AccentButton("This frame", onFrame, icon = ConcatIcons.Frame)
                    GhostButton("From photos", onPicture, icon = ConcatIcons.Image)
                }
                if (named) GhostButton("Use the edit's", onReset, icon = ConcatIcons.Reset)
            }
        }
    }
}
