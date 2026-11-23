// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.projects

import android.app.Application
import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import app.concat.editor.spike.api.ProjectInfo
import app.concat.editor.spike.concat
import app.concat.editor.spike.ui.Palette
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class ProjectsUi(
    val projects: List<ProjectInfo> = emptyList(),
    val loading: Boolean = true,
    val message: String? = null,
    val fake: Boolean = false,
)

/** The aspect choices for a new project, as frame sizes. */
enum class Aspect(val label: String, val width: Int, val height: Int) {
    Portrait("9:16", 1080, 1920),
    Landscape("16:9", 1920, 1080),
    Square("1:1", 1080, 1080),
}

class ProjectsViewModel(application: Application) : AndroidViewModel(application) {
    private val app = application.concat
    private val _ui = MutableStateFlow(ProjectsUi())
    val ui: StateFlow<ProjectsUi> = _ui

    fun reload() {
        viewModelScope.launch {
            try {
                val list = app.api.listProjects()
                _ui.update { it.copy(projects = list, loading = false, fake = app.api.engine.isFake) }
            } catch (error: Exception) {
                _ui.update { it.copy(loading = false, message = error.message) }
            }
        }
    }

    fun create(name: String, aspect: Aspect, onOpen: (String) -> Unit) {
        viewModelScope.launch {
            try {
                val path = app.api.createProject(app.projectsDir.absolutePath, name.trim(), aspect.width, aspect.height, 30)
                onOpen(path)
            } catch (error: Exception) {
                _ui.update { it.copy(message = error.message) }
            }
        }
    }

    fun dismissMessage() = _ui.update { it.copy(message = null) }
}

@Composable
fun ProjectsScreen(onOpen: (String) -> Unit, onSettings: () -> Unit = {}, model: ProjectsViewModel = viewModel()) {
    val ui by model.ui.collectAsStateWithLifecycle()
    var query by rememberSaveable { mutableStateOf("") }
    var creating by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { model.reload() }

    Column(
        Modifier.fillMaxSize().background(Palette.background).safeDrawingPadding().padding(horizontal = 16.dp),
    ) {
        Spacer(Modifier.height(12.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Concat", fontSize = 28.sp, fontWeight = FontWeight.Bold, color = Palette.text, modifier = Modifier.weight(1f))
            IconButton(onClick = onSettings) {
                Icon(Icons.Filled.Settings, contentDescription = "Settings", tint = Palette.text)
            }
        }
        if (ui.fake) {
            Text("In-memory engine: built without the native library", color = Palette.muted, fontSize = 12.sp)
        }
        Spacer(Modifier.height(16.dp))
        Box(
            Modifier.fillMaxWidth().height(120.dp).clip(RoundedCornerShape(16.dp))
                .background(Palette.raised).clickable { creating = true },
            contentAlignment = Alignment.Center,
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    Modifier.size(44.dp).clip(RoundedCornerShape(22.dp)).background(Palette.accent),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Filled.Add, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimary) }
                Spacer(Modifier.width(12.dp))
                Text("New project", fontSize = 18.sp, fontWeight = FontWeight.SemiBold, color = Palette.text)
            }
        }
        Spacer(Modifier.height(16.dp))
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
            placeholder = { Text("Search projects") },
            shape = RoundedCornerShape(12.dp),
        )
        Spacer(Modifier.height(12.dp))
        Text("Recent", color = Palette.muted, fontSize = 13.sp)
        Spacer(Modifier.height(8.dp))
        val shown = ui.projects.filter { query.isBlank() || it.name.contains(query.trim(), ignoreCase = true) }
        if (!ui.loading && shown.isEmpty()) {
            Text(
                if (ui.projects.isEmpty()) "No projects yet. Make one above." else "Nothing matches \"$query\".",
                color = Palette.muted,
                modifier = Modifier.padding(vertical = 24.dp),
            )
        }
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.weight(1f)) {
            items(shown, key = { it.path }) { project -> ProjectRow(project) { onOpen(project.path) } }
        }
    }

    if (creating) {
        NewProjectDialog(
            onDismiss = { creating = false },
            onCreate = { name, aspect ->
                creating = false
                model.create(name, aspect, onOpen)
            },
        )
    }
    ui.message?.let { message ->
        AlertDialog(
            onDismissRequest = model::dismissMessage,
            confirmButton = { TextButton(onClick = model::dismissMessage) { Text("OK") } },
            text = { Text(message) },
        )
    }
}

@Composable
private fun ProjectRow(project: ProjectInfo, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Palette.surface).clickable(onClick = onClick).padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val aspect = if (project.height > 0) project.width.toFloat() / project.height else 9f / 16f
        Box(Modifier.height(56.dp).aspectRatio(aspect.coerceIn(0.4f, 2f)).clip(RoundedCornerShape(6.dp)).background(Palette.raised))
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(project.name, color = Palette.text, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val fps = project.rateNum.toDouble() / project.rateDen
            val opened = if (project.openedAt > 0) DateUtils.getRelativeTimeSpanString(project.openedAt).toString() else ""
            Text(
                "${project.width}x${project.height} - ${"%.0f".format(fps)} fps" + if (opened.isNotEmpty()) " - $opened" else "",
                color = Palette.muted,
                fontSize = 12.sp,
            )
        }
    }
}

@Composable
private fun NewProjectDialog(onDismiss: () -> Unit, onCreate: (String, Aspect) -> Unit) {
    // A name of its own by default: a second "Untitled" would be refused as a folder that exists.
    var name by remember { mutableStateOf("Project " + SimpleDateFormat("MMM d HH.mm", Locale.getDefault()).format(Date())) }
    var aspect by remember { mutableStateOf(Aspect.Portrait) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New project") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    singleLine = true,
                    label = { Text("Name") },
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (option in Aspect.entries) {
                        FilterChip(selected = aspect == option, onClick = { aspect = option }, label = { Text(option.label) })
                    }
                }
                Text("30 fps", color = Palette.muted, fontSize = 12.sp)
            }
        },
        confirmButton = {
            Button(onClick = { onCreate(name.ifBlank { "Untitled" }, aspect) }) { Text("Create") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
