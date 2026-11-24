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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
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
import app.concat.editor.api.JobTracker
import app.concat.editor.api.errorMessage
import app.concat.editor.api.wasCancelled
import app.concat.editor.concat
import app.concat.editor.ui.Palette
import app.concat.editor.ui.components.ToolPanel
import app.concat.editor.ui.theme.ConcatColors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONObject

data class SpeechUi(
    val text: String = "",
    val model: String? = null,
    val voice: Int? = null,
    val speed: Float = 1f,
    val running: Boolean = false,
    val stage: String = "",
    val fraction: Float = 0f,
    val error: String? = null,
)

/**
 * Text read aloud: `speech.speak` as a job, then the file it wrote
 * imported and placed at the playhead as it was when Speak was pressed.
 */
class SpeechViewModel(application: Application, private val path: String) : AndroidViewModel(application) {
    private val api = application.concat.api
    private val _ui = MutableStateFlow(SpeechUi())
    val ui: StateFlow<SpeechUi> = _ui
    private val tracker = JobTracker("speech", ::handle)
    private var editor: EditorViewModel? = null
    private var at = 0.0
    private var finished: (() -> Unit)? = null

    init {
        viewModelScope.launch { api.rawEvents.collect { tracker.offer(it) } }
    }

    private fun handle(event: JSONObject) {
        when (event.optString("event")) {
            "speech.progress" -> _ui.update { it.copy(stage = event.optString("stage"), fraction = event.optDouble("fraction", 0.0).toFloat()) }
            "speech.done" -> {
                tracker.stop()
                _ui.update { it.copy(running = false, text = "") }
                editor?.placeSound(event.optString("file"), at, "Read aloud")
                finished?.invoke()
                finished = null
            }
            "speech.failed" -> {
                tracker.stop()
                _ui.update { it.copy(running = false, error = if (event.wasCancelled()) null else event.errorMessage()) }
            }
        }
    }

    fun setText(text: String) = _ui.update { it.copy(text = text) }
    fun chooseModel(id: String) = _ui.update { it.copy(model = id) }
    fun chooseVoice(id: Int) = _ui.update { it.copy(voice = id) }
    fun setSpeed(speed: Float) = _ui.update { it.copy(speed = speed) }

    fun speak(editor: EditorViewModel, done: () -> Unit) {
        val now = _ui.value
        val model = now.model ?: return
        val voice = now.voice ?: 0
        if (now.text.isBlank()) return
        this.editor = editor
        at = editor.ui.value.position
        finished = done
        _ui.update { it.copy(running = true, stage = "starting", fraction = 0f, error = null) }
        tracker.expect()
        viewModelScope.launch {
            try {
                tracker.start(api.speechSpeak(path, now.text.trim(), model, voice, now.speed.toDouble()))
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                tracker.stop()
                _ui.update { it.copy(running = false, error = error.message ?: "Reading did not start.") }
            }
        }
    }

    fun cancel() {
        val job = tracker.job ?: return
        finished = null
        viewModelScope.launch {
            try {
                api.speechCancel(job)
            } catch (error: Exception) {
                tracker.stop()
                _ui.update { it.copy(running = false) }
            }
        }
    }

    /** The sheet closed: what finishes still lands, the sheet just is not there to close. */
    fun detach() {
        finished = null
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SpeechSheet(speech: SpeechViewModel, editor: EditorViewModel, onDismiss: () -> Unit) {
    val ui by speech.ui.collectAsStateWithLifecycle()
    val store = editor.models
    val models by store.state.collectAsStateWithLifecycle()
    val model = models.info.speech.firstOrNull { it.id == ui.model }
    val voices = models.info.voices.filter { model == null || it.family == model.family }

    LaunchedEffect(Unit) { store.refresh() }
    LaunchedEffect(models.info.speech) {
        if (ui.model == null || models.info.speech.none { it.id == ui.model && it.downloaded }) {
            models.info.speech.firstOrNull { it.downloaded }?.let { speech.chooseModel(it.id) }
        }
    }
    LaunchedEffect(ui.model, voices) {
        if (voices.none { it.id == ui.voice }) voices.firstOrNull()?.let { speech.chooseVoice(it.id) }
    }
    val close = {
        speech.detach()
        onDismiss()
    }

    ToolPanel("Text to speech", onClose = close, scroll = false, padding = androidx.compose.foundation.layout.PaddingValues(0.dp)) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).imePadding().heightIn(max = 620.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ui.error?.let { Text(it, color = ConcatColors.warn, fontSize = 12.sp) }
            OutlinedTextField(
                value = ui.text,
                onValueChange = speech::setText,
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text("What should be said") },
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                minLines = 3,
                maxLines = 8,
                enabled = !ui.running,
            )
            SectionLabel("Model")
            ModelPicker(models.info.speech, ui.model, models.downloads, { speech.chooseModel(it.id) }, store::download)
            if (voices.isNotEmpty()) {
                SectionLabel("Voice")
                ChoiceChips(voices.map { it.id }, ui.voice ?: -1, { id -> voiceName(voices.first { it.id == id }.name) }, speech::chooseVoice)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                SectionLabel("Speed")
                Spacer(Modifier.weight(1f))
                Text("%.2fx".format(ui.speed), color = Palette.accent, fontSize = 12.sp)
            }
            Slider(value = ui.speed, onValueChange = speech::setSpeed, valueRange = 0.5f..2f, steps = 5)
            if (ui.running) {
                JobProgress(ui.stage, ui.fraction)
                OutlinedButton(onClick = speech::cancel, modifier = Modifier.fillMaxWidth()) { Text("Cancel") }
            } else {
                Button(
                    onClick = { speech.speak(editor, onDismiss) },
                    enabled = ui.text.isNotBlank() && model?.downloaded == true,
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Speak and place at playhead") }
            }
            Spacer(Modifier.navigationBarsPadding().padding(bottom = 8.dp))
        }
    }
}

/**
 * A voice's name as a person says it: "bf_ruth" is Ruth (the prefix is
 * its accent and gender), the phone's voice is the phone's.
 */
internal fun voiceName(name: String): String = when (name) {
    "system_default" -> "Phone's default"
    "chatterbox_clone" -> "The selected clip's voice"
    else -> name.substringAfter('_').replaceFirstChar { it.uppercase() }
}
