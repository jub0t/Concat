// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.export

import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.concat.editor.api.ApiEvent
import app.concat.editor.api.ConcatApi
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A choice on the export sheet. */
data class ExportPreset(val label: String, val shortSide: Int, val crf: Int, val preset: String)

val EXPORT_PRESETS = listOf(
    ExportPreset("720p", 720, 23, "veryfast"),
    ExportPreset("1080p", 1080, 20, "medium"),
    ExportPreset("1080p HQ", 1080, 17, "slow"),
    ExportPreset("4K", 2160, 20, "medium"),
)

sealed interface ExportState {
    data object Idle : ExportState
    data class Running(val job: String, val title: String, val stage: String, val fraction: Float) : ExportState
    data class Done(val title: String, val file: File, val published: Uri?, val note: String?) : ExportState
    data class Failed(val title: String, val message: String, val cancelled: Boolean) : ExportState
}

/**
 * The one export a process runs, as the sheet and the foreground service
 * both see it: `export.run`, then the job's events, then the file
 * published to Movies/Concat. Lives in the application, so it outlives
 * the editor screen and the activity.
 */
class ExportManager(private val context: Context, private val api: ConcatApi, private val scope: CoroutineScope) {
    private val _state = MutableStateFlow<ExportState>(ExportState.Idle)
    val state: StateFlow<ExportState> = _state

    /** Events for a job whose `export.run` reply has not come back yet. */
    private val early = HashMap<String, ApiEvent>()

    init {
        scope.launch { api.events.collect { handle(it) } }
    }

    private fun handle(event: ApiEvent) = synchronized(this) {
        val running = _state.value as? ExportState.Running
        if (running == null || event.job != running.job) {
            // An engine may report before the reply naming the job lands;
            // the last word on each job is kept for that moment.
            if (early.size > 8) early.clear()
            early[event.job] = event
            return@synchronized
        }
        when (event) {
            is ApiEvent.Progress -> _state.value = running.copy(
                stage = event.stage,
                fraction = if (event.total > 0) (event.frame.toFloat() / event.total).coerceIn(0f, 1f) else running.fraction,
            )
            is ApiEvent.Analysing -> _state.value = running.copy(stage = event.what, fraction = event.fraction.toFloat())
            is ApiEvent.Done -> finish(running.title, File(event.output))
            is ApiEvent.Failed -> _state.value =
                ExportState.Failed(running.title, event.message, cancelled = event.code == "cancelled")
        }
    }

    /** Starts rendering [path] at [preset]; the frame keeps the timeline's shape. */
    suspend fun start(path: String, title: String, width: Int, height: Int, preset: ExportPreset): Result<Unit> {
        if (_state.value is ExportState.Running) return Result.failure(IllegalStateException("An export is already running."))
        val (w, h) = sized(width, height, preset.shortSide)
        val dir = File(context.cacheDir, "exports").apply { mkdirs() }
        val safe = title.replace(Regex("[^A-Za-z0-9 _-]"), "_").ifBlank { "Concat" }
        val output = File(dir, "$safe-${System.currentTimeMillis()}.mp4")
        return try {
            val job = api.exportRun(path, output.absolutePath, w, h, preset.crf, preset.preset)
            val caught = synchronized(this) {
                _state.value = ExportState.Running(job, title, "starting", 0f)
                early.remove(job)
            }
            caught?.let(::handle)
            ContextCompat.startForegroundService(context, Intent(context, ExportService::class.java))
            Result.success(Unit)
        } catch (error: Exception) {
            Result.failure(error)
        }
    }

    fun cancel() {
        val running = _state.value as? ExportState.Running ?: return
        scope.launch {
            try {
                api.exportCancel(running.job)
            } catch (error: Exception) {
                Log.w(TAG, "cancel refused", error)
            }
        }
    }

    /** Back to nothing, once the sheet has shown how it ended. */
    fun dismiss() {
        if (_state.value !is ExportState.Running) _state.value = ExportState.Idle
    }

    private fun finish(title: String, file: File) {
        scope.launch {
            val (uri, note) = withContext(Dispatchers.IO) {
                try {
                    publish(file, title) to null
                } catch (error: Exception) {
                    Log.w(TAG, "could not publish ${file.name}", error)
                    null to "Saved in the app only: ${error.message}"
                }
            }
            _state.value = ExportState.Done(title, file, uri, note)
        }
    }

    /** Copies the file into Movies/Concat, where the gallery finds it. */
    private fun publish(file: File, title: String): Uri {
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Video.Media.DISPLAY_NAME, file.name)
            put(MediaStore.Video.Media.TITLE, title)
            put(MediaStore.Video.Media.MIME_TYPE, "video/mp4")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                put(MediaStore.Video.Media.RELATIVE_PATH, "${Environment.DIRECTORY_MOVIES}/Concat")
                put(MediaStore.Video.Media.IS_PENDING, 1)
            } else {
                // Before scoped storage the row names its file, which needs
                // WRITE_EXTERNAL_STORAGE (declared up to API 28).
                @Suppress("DEPRECATION")
                val movies = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_MOVIES), "Concat").apply { mkdirs() }
                @Suppress("DEPRECATION")
                put(MediaStore.Video.Media.DATA, File(movies, file.name).absolutePath)
            }
        }
        val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        } else {
            MediaStore.Video.Media.EXTERNAL_CONTENT_URI
        }
        val uri = resolver.insert(collection, values) ?: error("the media store made no entry")
        try {
            resolver.openOutputStream(uri)?.use { out -> file.inputStream().use { it.copyTo(out, 1 shl 20) } }
                ?: error("the media store would not open the entry")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                resolver.update(uri, ContentValues().apply { put(MediaStore.Video.Media.IS_PENDING, 0) }, null, null)
            }
        } catch (error: Exception) {
            resolver.delete(uri, null, null)
            throw error
        }
        return uri
    }

    /** The share sheet for a finished export: its gallery entry, or the app's own copy. */
    fun shareIntent(done: ExportState.Done): Intent {
        val uri = done.published
            ?: FileProvider.getUriForFile(context, "${context.packageName}.files", done.file)
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "video/mp4"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        return Intent.createChooser(send, "Share video")
    }

    private companion object {
        const val TAG = "concat-export"

        /** The timeline's shape with its short side at [short], both even. */
        fun sized(width: Int, height: Int, short: Int): Pair<Int, Int> {
            if (width <= 0 || height <= 0) return 1080 to 1920
            val scale = short.toDouble() / minOf(width, height)
            fun even(v: Double) = (Math.round(v / 2) * 2).toInt().coerceAtLeast(2)
            return even(width * scale) to even(height * scale)
        }
    }
}
