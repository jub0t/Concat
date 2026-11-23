// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.models

import android.util.Log
import app.concat.editor.spike.api.ConcatApi
import app.concat.editor.spike.api.ModelInfo
import app.concat.editor.spike.api.ModelsInfo
import app.concat.editor.spike.api.errorMessage
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONObject

/** How far one model's download is. */
data class ModelDownload(val received: Long, val total: Long, val unpacking: Boolean) {
    val fraction: Float get() = if (total > 0) (received.toFloat() / total).coerceIn(0f, 1f) else 0f
}

data class ModelsState(
    val info: ModelsInfo = ModelsInfo.EMPTY,
    val loaded: Boolean = false,
    /** Running downloads, by model id. */
    val downloads: Map<String, ModelDownload> = emptyMap(),
    val error: String? = null,
)

/**
 * The speech models as every screen sees them: `models.list`, and the
 * downloads running, from `models.progress` / `models.done` /
 * `models.failed`. Lives in the application, so a download started on the
 * captions sheet carries on when it closes and shows in Settings.
 */
class ModelStore(private val api: ConcatApi, private val scope: CoroutineScope) {
    private val _state = MutableStateFlow(ModelsState())
    val state: StateFlow<ModelsState> = _state

    init {
        scope.launch { api.rawEvents.collect(::handle) }
    }

    private fun handle(event: JSONObject) {
        val id = event.optString("id")
        when (event.optString("event")) {
            "models.progress" -> _state.update {
                it.copy(downloads = it.downloads + (id to ModelDownload(event.optLong("received"), event.optLong("total"), event.optBoolean("unpacking"))))
            }
            "models.done" -> {
                _state.update { it.copy(downloads = it.downloads - id) }
                refresh()
            }
            "models.failed" -> _state.update { it.copy(downloads = it.downloads - id, error = event.errorMessage()) }
        }
    }

    fun refresh() {
        scope.launch {
            try {
                val info = api.models()
                _state.update { it.copy(info = info, loaded = true) }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                Log.w(TAG, "models.list failed", error)
                _state.update { it.copy(loaded = true, error = error.message) }
            }
        }
    }

    /** "transcriber" for a captions model, "speech" for a reading one. */
    fun kindOf(model: ModelInfo): String = if (_state.value.info.transcribers.any { it.id == model.id }) "transcriber" else "speech"

    fun download(model: ModelInfo) {
        if (model.id in _state.value.downloads) return
        _state.update { it.copy(downloads = it.downloads + (model.id to ModelDownload(0, model.sizeBytes, false))) }
        scope.launch {
            try {
                api.modelsDownload(kindOf(model), model.id)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _state.update { it.copy(downloads = it.downloads - model.id, error = error.message) }
            }
        }
    }

    fun delete(model: ModelInfo) {
        scope.launch {
            try {
                api.modelsDelete(kindOf(model), model.id)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _state.update { it.copy(error = error.message) }
            }
            refresh()
        }
    }

    fun dismissError() = _state.update { it.copy(error = null) }

    private companion object {
        const val TAG = "concat-models"
    }
}

/** "142 MB". */
fun sizeLabel(bytes: Long): String = when {
    bytes <= 0 -> "built in"
    bytes >= 1_000_000_000 -> "%.1f GB".format(bytes / 1e9)
    else -> "${(bytes / 1_000_000).coerceAtLeast(1)} MB"
}
