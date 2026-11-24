// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui.editor

import android.app.Application
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.concat.editor.api.CaptionSegment
import app.concat.editor.api.Clip
import app.concat.editor.api.JobTracker
import app.concat.editor.api.errorMessage
import app.concat.editor.api.wasCancelled
import app.concat.editor.concat
import app.concat.editor.ui.Palette
import app.concat.editor.ui.components.ToolPanel
import app.concat.editor.ui.theme.ConcatColors
import app.concat.editor.ui.timecode
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONObject

/** The languages offered; null is "find it by ear". */
val CAPTION_LANGUAGES: List<Pair<String?, String>> = listOf(
    null to "Auto", "en" to "English", "es" to "Spanish", "fr" to "French", "de" to "German",
    "pt" to "Portuguese", "it" to "Italian", "nl" to "Dutch", "ar" to "Arabic", "hi" to "Hindi",
    "ur" to "Urdu", "tr" to "Turkish", "ru" to "Russian", "ja" to "Japanese", "ko" to "Korean", "zh" to "Chinese",
)

enum class CaptionsStep { Setup, Hearing, Review }

data class CaptionsUi(
    val step: CaptionsStep = CaptionsStep.Setup,
    val clipId: String? = null,
    val model: String? = null,
    val language: String? = null,
    val placement: String = "bottom",
    val size: String = "medium",
    val stage: String = "",
    val fraction: Float = 0f,
    val segments: List<CaptionSegment> = emptyList(),
    val error: String? = null,
)

/**
 * Captions for one project: `captions.transcribe` as a job, its words
 * held for correcting, then `captions.place` through the editor, one undo
 * step. The job outlives the sheet: close it while hearing and open it
 * again to find it still going.
 */
class CaptionsViewModel(application: Application, private val path: String) : AndroidViewModel(application) {
    private val api = application.concat.api
    private val _ui = MutableStateFlow(CaptionsUi())
    val ui: StateFlow<CaptionsUi> = _ui
    private val tracker = JobTracker("captions", ::handle)

    init {
        viewModelScope.launch { api.rawEvents.collect { tracker.offer(it) } }
    }

    private fun handle(event: JSONObject) {
        when (event.optString("event")) {
            "captions.progress" -> _ui.update { it.copy(stage = event.optString("stage"), fraction = event.optDouble("fraction", 0.0).toFloat()) }
            "captions.done" -> {
                tracker.stop()
                val segments = CaptionSegment.list(event.optJSONArray("segments"))
                _ui.update {
                    it.copy(
                        step = if (segments.isEmpty()) CaptionsStep.Setup else CaptionsStep.Review,
                        segments = segments,
                        error = if (segments.isEmpty()) "Nothing was said in that clip." else null,
                    )
                }
            }
            "captions.failed" -> {
                tracker.stop()
                _ui.update { it.copy(step = CaptionsStep.Setup, error = if (event.wasCancelled()) null else event.errorMessage()) }
            }
        }
    }

    fun chooseClip(id: String) = _ui.update { if (it.step == CaptionsStep.Setup) it.copy(clipId = id) else it }
    fun chooseModel(id: String) = _ui.update { it.copy(model = id) }
    fun chooseLanguage(code: String?) = _ui.update { it.copy(language = code) }
    fun choosePlacement(value: String) = _ui.update { it.copy(placement = value) }
    fun chooseSize(value: String) = _ui.update { it.copy(size = value) }
    fun dismissError() = _ui.update { it.copy(error = null) }

    fun transcribe() {
        val now = _ui.value
        val clip = now.clipId ?: return
        val model = now.model ?: return
        _ui.update { it.copy(step = CaptionsStep.Hearing, stage = "starting", fraction = 0f, error = null) }
        tracker.expect()
        viewModelScope.launch {
            try {
                tracker.start(api.captionsTranscribe(path, clip, model, now.language))
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                tracker.stop()
                _ui.update { it.copy(step = CaptionsStep.Setup, error = error.message ?: "Captions did not start.") }
            }
        }
    }

    fun cancel() {
        val job = tracker.job
        if (job == null) {
            tracker.stop()
            _ui.update { it.copy(step = CaptionsStep.Setup) }
            return
        }
        viewModelScope.launch {
            try {
                api.captionsCancel(job)
            } catch (error: Exception) {
                tracker.stop()
                _ui.update { it.copy(step = CaptionsStep.Setup) }
            }
        }
    }

    fun editSegment(index: Int, text: String) = _ui.update { ui ->
        ui.copy(segments = ui.segments.mapIndexed { i, segment -> if (i == index) segment.copy(text = text) else segment })
    }

    fun discard() = _ui.update { it.copy(step = CaptionsStep.Setup, segments = emptyList()) }

    /** The words, as corrected, over their clip as titles. */
    fun place(editor: EditorViewModel, done: () -> Unit) {
        val now = _ui.value
        val clip = now.clipId ?: return
        val segments = now.segments.filter { it.text.isNotBlank() }
        if (segments.isEmpty()) return
        editor.edit("Captions") { api.captionsPlace(path, clip, segments, now.placement, now.size) }
        _ui.update { it.copy(step = CaptionsStep.Setup, segments = emptyList()) }
        done()
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CaptionsSheet(captions: CaptionsViewModel, editor: EditorViewModel, preferClip: String?, onDismiss: () -> Unit) {
    val ui by captions.ui.collectAsStateWithLifecycle()
    val editorUi by editor.ui.collectAsStateWithLifecycle()
    val store = editor.models
    val models by store.state.collectAsStateWithLifecycle()
    val view = editorUi.view
    val sound: List<Clip> = view?.clips?.filter { it.kind.hasSound } ?: emptyList()

    LaunchedEffect(Unit) { store.refresh() }
    LaunchedEffect(preferClip, sound.size) {
        val wanted = preferClip?.takeIf { id -> sound.any { it.id == id } } ?: ui.clipId?.takeIf { id -> sound.any { it.id == id } }
        (wanted ?: sound.firstOrNull()?.id)?.let(captions::chooseClip)
    }
    LaunchedEffect(models.info.transcribers) {
        if (ui.model == null || models.info.transcribers.none { it.id == ui.model && it.downloaded }) {
            models.info.transcribers.firstOrNull { it.downloaded }?.let { captions.chooseModel(it.id) }
        }
    }

    ToolPanel("Auto captions", onClose = onDismiss, scroll = false, padding = androidx.compose.foundation.layout.PaddingValues(0.dp)) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).imePadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ui.error?.let { Text(it, color = ConcatColors.warn, fontSize = 12.sp) }
            when (ui.step) {
                CaptionsStep.Setup -> Column(
                    Modifier.heightIn(max = 520.dp).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (sound.isEmpty()) {
                        SectionLabel("Add a video or a sound with speech in it first.")
                    } else {
                        SectionLabel("Clip")
                        ChoiceChips(sound.map { it.id }, ui.clipId ?: "", { id ->
                            val clip = sound.first { it.id == id }
                            "${clip.name.ifBlank { clip.kind.name }.take(18)} at ${timecode(clip.start, view?.fps ?: 30.0)}"
                        }, captions::chooseClip)
                    }
                    SectionLabel("Speech model")
                    ModelPicker(models.info.transcribers, ui.model, models.downloads, { captions.chooseModel(it.id) }, store::download)
                    val englishOnly = models.info.transcribers.firstOrNull { it.id == ui.model }?.englishOnly == true
                    if (!englishOnly) {
                        SectionLabel("Language")
                        ChoiceChips(CAPTION_LANGUAGES, CAPTION_LANGUAGES.first { it.first == ui.language }, { it.second }) {
                            captions.chooseLanguage(it.first)
                        }
                    }
                    SectionLabel("Placement")
                    ChoiceChips(listOf("top", "middle", "bottom"), ui.placement, { it.replaceFirstChar(Char::uppercase) }, captions::choosePlacement)
                    SectionLabel("Size")
                    ChoiceChips(listOf("small", "medium", "large"), ui.size, { it.replaceFirstChar(Char::uppercase) }, captions::chooseSize)
                    Button(
                        onClick = captions::transcribe,
                        enabled = ui.clipId != null && models.info.transcribers.any { it.id == ui.model && it.downloaded },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text("Transcribe") }
                }
                CaptionsStep.Hearing -> {
                    JobProgress(ui.stage, ui.fraction)
                    SectionLabel("It carries on if you close this panel.")
                    OutlinedButton(onClick = captions::cancel, modifier = Modifier.fillMaxWidth()) { Text("Cancel") }
                }
                CaptionsStep.Review -> {
                    SectionLabel("${ui.segments.size} lines heard. Correct any before placing them.")
                    LazyColumn(Modifier.fillMaxWidth().heightIn(max = 380.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        itemsIndexed(ui.segments) { index, segment ->
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    timecode(segment.start, view?.fps ?: 30.0),
                                    color = Palette.muted,
                                    fontSize = 11.sp,
                                    modifier = Modifier.padding(end = 8.dp),
                                )
                                OutlinedTextField(
                                    value = segment.text,
                                    onValueChange = { captions.editSegment(index, it) },
                                    modifier = Modifier.weight(1f),
                                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                                    maxLines = 3,
                                )
                            }
                        }
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton(onClick = captions::discard, modifier = Modifier.weight(1f)) { Text("Discard") }
                        Button(
                            onClick = { captions.place(editor, onDismiss) },
                            enabled = ui.segments.any { it.text.isNotBlank() },
                            modifier = Modifier.weight(1f),
                        ) { Text("Place") }
                    }
                }
            }
            Spacer(Modifier.navigationBarsPadding().padding(bottom = 8.dp))
        }
    }
}
