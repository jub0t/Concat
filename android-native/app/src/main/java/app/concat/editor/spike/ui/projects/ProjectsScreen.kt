// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.projects

import android.app.Application
import android.graphics.BitmapFactory
import android.text.format.DateUtils
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import app.concat.editor.spike.api.ProjectInfo
import app.concat.editor.spike.concat
import app.concat.editor.spike.ui.components.ChoiceChip
import app.concat.editor.spike.ui.components.ConcatIcon
import app.concat.editor.spike.ui.components.IconAction
import app.concat.editor.spike.ui.icons.ConcatIcons
import app.concat.editor.spike.ui.secondsLabel
import app.concat.editor.spike.ui.theme.ConcatColors
import app.concat.editor.spike.ui.theme.ConcatType
import app.concat.editor.spike.ui.theme.Radii
import app.concat.editor.spike.ui.theme.Spacing
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** One project's card: its recents entry, its frame and its length when the editor left them. */
data class ProjectCard(val info: ProjectInfo, val thumbnail: ImageBitmap?, val duration: Double?)

data class ProjectsUi(
    val projects: List<ProjectCard> = emptyList(),
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

private const val THUMBNAIL = ".concat-thumb.png"
private const val CARD = ".concat-card.json"

class ProjectsViewModel(application: Application) : AndroidViewModel(application) {
    private val app = application.concat
    private val _ui = MutableStateFlow(ProjectsUi())
    val ui: StateFlow<ProjectsUi> = _ui

    fun reload() {
        viewModelScope.launch {
            try {
                val list = app.api.listProjects()
                val cards = withContext(Dispatchers.IO) {
                    list.filter { File(it.path).isDirectory }.map { info -> card(info) }
                }
                _ui.update { it.copy(projects = cards, loading = false, fake = app.api.engine.isFake) }
            } catch (error: Exception) {
                _ui.update { it.copy(loading = false, message = error.message) }
            }
        }
    }

    /** The frame the editor left in the folder; the in-memory engine paints one on asking. */
    private suspend fun card(info: ProjectInfo): ProjectCard {
        val folder = File(info.path)
        val thumb = File(folder, THUMBNAIL)
        if (!thumb.exists() && app.api.engine.isFake) {
            try {
                app.api.previewFrameTo(info.path, 0.5, thumb.absolutePath, 270, (270.0 * info.height / info.width.coerceAtLeast(1)).toInt())
            } catch (_: Exception) {
            }
        }
        val image = if (thumb.exists()) {
            try {
                BitmapFactory.decodeFile(thumb.absolutePath)?.asImageBitmap()
            } catch (_: Exception) {
                null
            }
        } else {
            null
        }
        val duration = try {
            File(folder, CARD).takeIf { it.exists() }?.let { JSONObject(it.readText()).optDouble("duration").takeIf { d -> !d.isNaN() && d > 0 } }
        } catch (_: Exception) {
            null
        }
        return ProjectCard(info, image, duration)
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

    /** A new name: open, `project.save` with the name, close. */
    fun rename(project: ProjectInfo, name: String) {
        viewModelScope.launch {
            try {
                app.api.open(project.path)
                app.api.rename(project.path, name.trim())
                app.api.close(project.path, save = false)
            } catch (error: Exception) {
                _ui.update { it.copy(message = error.message) }
            }
            reload()
        }
    }

    /** The folder copied beside itself, then renamed as a project of its own. */
    fun duplicate(project: ProjectInfo) {
        viewModelScope.launch {
            try {
                val source = File(project.path)
                val name = uniqueName("${project.name} copy")
                val target = File(source.parentFile, name.replace(Regex("[^A-Za-z0-9 _.-]"), "_"))
                withContext(Dispatchers.IO) { source.copyRecursively(target, overwrite = false) }
                app.api.open(target.absolutePath)
                app.api.rename(target.absolutePath, name)
                app.api.close(target.absolutePath, save = false)
            } catch (error: Exception) {
                _ui.update { it.copy(message = error.message ?: "The copy did not work.") }
            }
            reload()
        }
    }

    private fun uniqueName(base: String): String {
        val taken = _ui.value.projects.map { it.info.name }.toSet()
        if (base !in taken) return base
        var n = 2
        while ("$base $n" in taken) n++
        return "$base $n"
    }

    fun delete(project: ProjectInfo) {
        viewModelScope.launch {
            try {
                try {
                    app.api.close(project.path, save = false)
                } catch (_: Exception) {
                }
                withContext(Dispatchers.IO) { File(project.path).deleteRecursively() }
            } catch (error: Exception) {
                _ui.update { it.copy(message = error.message) }
            }
            reload()
        }
    }

    fun dismissMessage() = _ui.update { it.copy(message = null) }
}

/**
 * Home: a big New project card, quick actions, and the projects as a grid
 * of cards with their frames, lengths and dates. A long press on a card
 * renames, duplicates or deletes it.
 */
@Composable
fun ProjectsScreen(
    onOpen: (path: String, tool: String?) -> Unit,
    onSettings: () -> Unit = {},
    model: ProjectsViewModel = viewModel(),
) {
    val ui by model.ui.collectAsStateWithLifecycle()
    var query by rememberSaveable { mutableStateOf("") }
    var searching by rememberSaveable { mutableStateOf(false) }
    var creating by remember { mutableStateOf<String?>(null) }
    var renaming by remember { mutableStateOf<ProjectInfo?>(null) }
    var deleting by remember { mutableStateOf<ProjectInfo?>(null) }
    LaunchedEffect(Unit) { model.reload() }
    val shown = ui.projects.filter { query.isBlank() || it.info.name.contains(query.trim(), ignoreCase = true) }

    LazyVerticalGrid(
        columns = GridCells.Fixed(2),
        modifier = Modifier.fillMaxSize().background(ConcatColors.page).safeDrawingPadding(),
        contentPadding = PaddingValues(horizontal = Spacing.lg, vertical = Spacing.md),
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
        verticalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        item(span = { GridItemSpan(maxLineSpan) }) {
            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Concat", style = ConcatType.display, color = ConcatColors.fg, modifier = Modifier.weight(1f))
                    IconAction(ConcatIcons.Settings, "Settings", onSettings)
                }
                if (ui.fake) {
                    Text("In-memory engine: built without the native library", style = ConcatType.caption, color = ConcatColors.fgDim)
                }
            }
        }
        item(span = { GridItemSpan(maxLineSpan) }) { NewProjectCard { creating = "" } }
        item(span = { GridItemSpan(maxLineSpan) }) {
            Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                QuickAction(ConcatIcons.Rows, "Auto captions", Modifier.weight(1f)) { creating = "captions" }
                QuickAction(ConcatIcons.Waveform, "Text to speech", Modifier.weight(1f)) { creating = "speech" }
                QuickAction(ConcatIcons.Slot, "Templates", Modifier.weight(1f), note = "Soon", enabled = false) {}
            }
        }
        item(span = { GridItemSpan(maxLineSpan) }) {
            Row(Modifier.padding(top = Spacing.sm), verticalAlignment = Alignment.CenterVertically) {
                Text("Projects", style = ConcatType.title, color = ConcatColors.fg)
                if (ui.projects.isNotEmpty()) {
                    Text("  ${ui.projects.size}", style = ConcatType.label, color = ConcatColors.fgDim)
                }
                Spacer(Modifier.weight(1f))
                IconAction(if (searching) ConcatIcons.Close else ConcatIcons.Search, if (searching) "Stop searching" else "Search", {
                    searching = !searching
                    if (!searching) query = ""
                }, size = 20.dp)
            }
        }
        if (searching) {
            item(span = { GridItemSpan(maxLineSpan) }) { SearchField(query) { query = it } }
        }
        if (!ui.loading && shown.isEmpty()) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Text(
                    if (ui.projects.isEmpty()) "No projects yet. Start one above." else "Nothing matches \"$query\".",
                    style = ConcatType.body,
                    color = ConcatColors.fgMuted,
                    modifier = Modifier.padding(vertical = Spacing.xl),
                )
            }
        }
        items(shown, key = { it.info.path }) { card ->
            ProjectTile(
                card,
                onOpen = { onOpen(card.info.path, null) },
                onRename = { renaming = card.info },
                onDuplicate = { model.duplicate(card.info) },
                onDelete = { deleting = card.info },
            )
        }
    }

    creating?.let { tool ->
        NewProjectDialog(
            onDismiss = { creating = null },
            onCreate = { name, aspect ->
                creating = null
                model.create(name, aspect) { path -> onOpen(path, tool.ifEmpty { null }) }
            },
        )
    }
    renaming?.let { project ->
        NameDialog("Rename project", project.name, "Rename", onDismiss = { renaming = null }) {
            model.rename(project, it)
            renaming = null
        }
    }
    deleting?.let { project ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            containerColor = ConcatColors.launchRaised,
            title = { Text("Delete \"${project.name}\"?", style = ConcatType.title) },
            text = { Text("The project and its folder go for good. Media picked from the phone stays on the phone.", style = ConcatType.body, color = ConcatColors.fgMuted) },
            confirmButton = {
                TextButton(onClick = {
                    model.delete(project)
                    deleting = null
                }) { Text("Delete", color = ConcatColors.danger) }
            },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Cancel") } },
        )
    }
    ui.message?.let { message ->
        AlertDialog(
            onDismissRequest = model::dismissMessage,
            containerColor = ConcatColors.launchRaised,
            confirmButton = { TextButton(onClick = model::dismissMessage) { Text("OK") } },
            text = { Text(message) },
        )
    }
}

@Composable
private fun NewProjectCard(onClick: () -> Unit) {
    Box(
        Modifier.fillMaxWidth().height(128.dp).clip(RoundedCornerShape(Radii.xl))
            .background(Brush.linearGradient(listOf(Color(0xFF2A3A10), ConcatColors.launchRaised, ConcatColors.launchPanel)))
            .border(1.dp, ConcatColors.line, RoundedCornerShape(Radii.xl))
            .clickable(onClickLabel = "New project", onClick = onClick),
        contentAlignment = Alignment.CenterStart,
    ) {
        Row(Modifier.padding(horizontal = Spacing.xl), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(56.dp).clip(CircleShape).background(ConcatColors.accent), contentAlignment = Alignment.Center) {
                ConcatIcon(ConcatIcons.Plus, tint = ConcatColors.onAccent, size = 28.dp)
            }
            Spacer(Modifier.width(Spacing.lg))
            Column {
                Text("New project", style = ConcatType.title.copy(fontSize = ConcatType.title.fontSize * 1.15f), color = ConcatColors.fg)
                Spacer(Modifier.height(2.dp))
                Text("Video, photos and sound, cut on the phone", style = ConcatType.label, color = ConcatColors.fgMuted)
            }
        }
    }
}

@Composable
private fun QuickAction(icon: ImageVector, label: String, modifier: Modifier, note: String? = null, enabled: Boolean = true, onClick: () -> Unit) {
    Column(
        modifier.height(88.dp).clip(RoundedCornerShape(Radii.lg)).background(ConcatColors.launchPanel)
            .clickable(enabled = enabled, onClick = onClick).padding(Spacing.md),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(32.dp).clip(CircleShape).background(ConcatColors.raised), contentAlignment = Alignment.Center) {
                ConcatIcon(icon, tint = if (enabled) ConcatColors.accent else ConcatColors.fgDim, size = 18.dp)
            }
            Spacer(Modifier.weight(1f))
            if (note != null) {
                Text(note, style = ConcatType.badge, color = ConcatColors.fgDim,
                    modifier = Modifier.clip(RoundedCornerShape(Radii.xs)).background(ConcatColors.raised).padding(horizontal = 5.dp, vertical = 2.dp))
            }
        }
        Text(label, style = ConcatType.label, color = if (enabled) ConcatColors.fg else ConcatColors.fgDim, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun SearchField(query: String, onChange: (String) -> Unit) {
    BasicTextField(
        value = query,
        onValueChange = onChange,
        singleLine = true,
        textStyle = ConcatType.body.copy(color = ConcatColors.fg),
        cursorBrush = SolidColor(ConcatColors.accent),
        decorationBox = { field ->
            Row(
                Modifier.fillMaxWidth().height(40.dp).clip(RoundedCornerShape(Radii.md)).background(ConcatColors.launchField).padding(horizontal = Spacing.md),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ConcatIcon(ConcatIcons.Search, tint = ConcatColors.fgDim, size = 18.dp)
                Spacer(Modifier.width(Spacing.sm))
                Box(Modifier.weight(1f)) {
                    if (query.isEmpty()) Text("Search projects", style = ConcatType.body, color = ConcatColors.fgDim)
                    field()
                }
            }
        },
    )
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ProjectTile(card: ProjectCard, onOpen: () -> Unit, onRename: () -> Unit, onDuplicate: () -> Unit, onDelete: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    val project = card.info
    Column(
        Modifier.clip(RoundedCornerShape(Radii.lg)).combinedClickable(
            onClick = onOpen,
            onLongClick = {
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                menu = true
            },
            onLongClickLabel = "More",
        ),
    ) {
        Box(Modifier.fillMaxWidth().aspectRatio(0.92f).clip(RoundedCornerShape(Radii.lg)).background(ConcatColors.launchPanel)) {
            if (card.thumbnail != null) {
                Image(card.thumbnail, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            } else {
                val hue = (project.name.hashCode() and 0xff) * 360f / 256f
                Box(
                    Modifier.fillMaxSize().background(Brush.linearGradient(listOf(Color.hsv(hue, 0.35f, 0.32f), Color.hsv((hue + 40) % 360f, 0.4f, 0.14f)))),
                    contentAlignment = Alignment.Center,
                ) { ConcatIcon(ConcatIcons.Film, tint = Color.White.copy(alpha = 0.5f), size = 28.dp) }
            }
            card.duration?.let {
                Text(
                    secondsLabel(it),
                    style = ConcatType.badge.copy(fontFeatureSettings = "tnum"),
                    color = Color.White,
                    modifier = Modifier.align(Alignment.BottomEnd).padding(Spacing.sm)
                        .clip(RoundedCornerShape(Radii.xs)).background(Color.Black.copy(alpha = 0.6f)).padding(horizontal = 5.dp, vertical = 2.dp),
                )
            }
            Text(
                "${project.width}x${project.height}",
                style = ConcatType.badge,
                color = Color.White.copy(alpha = 0.85f),
                modifier = Modifier.align(Alignment.TopStart).padding(Spacing.sm)
                    .clip(RoundedCornerShape(Radii.xs)).background(Color.Black.copy(alpha = 0.45f)).padding(horizontal = 5.dp, vertical = 2.dp),
            )
            Box(Modifier.align(Alignment.TopEnd)) {
                IconAction(ConcatIcons.More, "More", { menu = true }, tint = Color.White, size = 18.dp, box = 36.dp)
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = ConcatColors.launchRaised) {
                    DropdownMenuItem(text = { Text("Rename") }, leadingIcon = { ConcatIcon(ConcatIcons.TextMark, size = 18.dp) }, onClick = {
                        menu = false
                        onRename()
                    })
                    DropdownMenuItem(text = { Text("Duplicate") }, leadingIcon = { ConcatIcon(ConcatIcons.Copy, size = 18.dp) }, onClick = {
                        menu = false
                        onDuplicate()
                    })
                    DropdownMenuItem(text = { Text("Delete", color = ConcatColors.danger) }, leadingIcon = { ConcatIcon(ConcatIcons.Trash, tint = ConcatColors.danger, size = 18.dp) }, onClick = {
                        menu = false
                        onDelete()
                    })
                }
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(project.name, style = ConcatType.label.copy(fontSize = ConcatType.body.fontSize), color = ConcatColors.fg, maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(horizontal = 2.dp))
        Text(openedLabel(project.openedAt), style = ConcatType.caption, color = ConcatColors.fgDim, modifier = Modifier.padding(horizontal = 2.dp, vertical = 1.dp))
    }
}

/** When it was last opened: recents give milliseconds (the in-memory engine) or seconds (the engine). */
private fun openedLabel(openedAt: Long): String {
    if (openedAt <= 0) return ""
    val ms = if (openedAt < 100_000_000_000L) openedAt * 1000 else openedAt
    return DateUtils.getRelativeTimeSpanString(ms, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString()
}

@Composable
private fun NewProjectDialog(onDismiss: () -> Unit, onCreate: (String, Aspect) -> Unit) {
    // A name of its own by default: a second "Untitled" would be refused as a folder that exists.
    var name by remember { mutableStateOf("Project " + SimpleDateFormat("MMM d HH.mm", Locale.getDefault()).format(Date())) }
    var aspect by remember { mutableStateOf(Aspect.Portrait) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = ConcatColors.launchRaised,
        title = { Text("New project", style = ConcatType.title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
                OutlinedTextField(value = name, onValueChange = { name = it }, singleLine = true, label = { Text("Name") })
                Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
                    for (option in Aspect.entries) {
                        ChoiceChip(option.label, selected = aspect == option, onClick = { aspect = option })
                    }
                }
                Text("30 fps", style = ConcatType.caption, color = ConcatColors.fgDim)
            }
        },
        confirmButton = { TextButton(onClick = { onCreate(name.ifBlank { "Untitled" }, aspect) }) { Text("Create") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun NameDialog(title: String, current: String, action: String, onDismiss: () -> Unit, onDone: (String) -> Unit) {
    var name by remember { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = ConcatColors.launchRaised,
        title = { Text(title, style = ConcatType.title) },
        text = { OutlinedTextField(value = name, onValueChange = { name = it }, singleLine = true) },
        confirmButton = { TextButton(onClick = { onDone(name) }, enabled = name.isNotBlank()) { Text(action) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
