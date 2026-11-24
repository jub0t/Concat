// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor

import android.util.Log
import app.concat.editor.api.Commands
import app.concat.editor.export.EXPORT_PRESETS
import app.concat.editor.export.ExportState
import java.io.File
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The export CI's emulators drive (scripts/ci/smoke-android.sh --export):
 * only in a build made with `-Psmoke=true`, and only when a clip named
 * `smoke-export.*` waits in the app's files folder or its media folder.
 * It makes a project, imports the clip, puts it on the timeline and exports
 * it through the same calls the screens make, then says how it ended in
 * the log under the tag the script reads.
 */
object Smoke {
    private const val TAG = "concat"
    private var ran = false

    /** Once a process, from the activity's first start. */
    fun run(app: ConcatApplication) {
        if (!BuildConfig.SMOKE || ran) return
        ran = true
        // Asking for the files folder makes it, which the script waits for.
        val files = app.getExternalFilesDir(null)
        val folders = listOfNotNull(files, app.externalMediaDirs.firstOrNull())
        val clip = folders.firstNotNullOfOrNull { folder ->
            folder.listFiles()?.firstOrNull { it.isFile && it.nameWithoutExtension == "smoke-export" }
        }
        if (clip == null) {
            Log.i(TAG, "smoke: no smoke-export clip in ${folders.joinToString()}; an ordinary launch")
            return
        }
        app.scope.launch {
            try {
                Log.i(TAG, "smoke: exporting ${clip.absolutePath}")
                File(app.projectsDir, "smoke").deleteRecursively()
                val path = app.api.createProject(app.projectsDir.absolutePath, "smoke", 1080, 1920, 30)
                val imported = app.api.import(path, clip.absolutePath)
                val media = imported.media.firstOrNull { it.path == clip.absolutePath } ?: imported.media.first()
                app.api.apply(path, Commands.addClipsInSequence(listOf(media.id), 0.0))
                app.api.save(path)
                app.exports.start(path, "smoke", 1080, 1920, EXPORT_PRESETS.first()).getOrThrow()
                when (val end = app.exports.state.first { it is ExportState.Done || it is ExportState.Failed }) {
                    is ExportState.Done -> Log.i(TAG, "smoke: export finished: ${end.file} (${end.file.length()} bytes)")
                    is ExportState.Failed -> Log.e(TAG, "smoke: the export run failed: ${end.message}")
                    else -> Unit
                }
            } catch (error: Exception) {
                Log.e(TAG, "smoke: the export run failed: ${error.message}", error)
            }
        }
    }
}
