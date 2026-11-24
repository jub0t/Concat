// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.engine

import kotlinx.coroutines.flow.MutableStateFlow

/**
 * Whether the engine is running a job (an export, a transcription, a
 * download) that the screen should stay on for. The engine sets it from
 * whichever thread starts the first job or ends the last; the activity
 * keeps its window's screen on while it is true.
 */
object Wakefulness {
    val held = MutableStateFlow(false)
}
