// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.ui.editor

import android.app.Application
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import app.concat.editor.ui.icons.ConcatIcons
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.concat.editor.api.AppliedEffect
import app.concat.editor.api.Clip
import app.concat.editor.api.ClipKind
import app.concat.editor.api.PackageInfo
import app.concat.editor.api.ParamInfo
import app.concat.editor.concat
import app.concat.editor.ui.Palette
import kotlin.math.roundToInt
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONObject

data class CatalogueUi(val packages: List<PackageInfo> = emptyList(), val loaded: Boolean = false, val error: String? = null)

/** Every package the build knows (`catalogue.list`), read once per editor. */
class CatalogueViewModel(application: Application) : AndroidViewModel(application) {
    private val api = application.concat.api
    private val _ui = MutableStateFlow(CatalogueUi())
    val ui: StateFlow<CatalogueUi> = _ui

    fun load() {
        if (_ui.value.loaded && _ui.value.error == null) return
        viewModelScope.launch {
            try {
                val packages = api.catalogue()
                _ui.update { CatalogueUi(packages, loaded = true) }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _ui.update { it.copy(loaded = true, error = error.message ?: "The catalogue would not load.") }
            }
        }
    }

    fun find(id: String): PackageInfo? = _ui.value.packages.firstOrNull { it.id == id }
}

/** Which packages a sheet shows, and what it is called. */
enum class Shelf(val title: String) { Effects("Effects"), Filters("Filters"), Transitions("Transitions") }

/** The package kinds a shelf offers for [clip]. */
fun Shelf.kinds(clip: Clip?): List<String> = when (this) {
    Shelf.Effects -> when (clip?.kind) {
        ClipKind.Audio -> listOf("audio")
        ClipKind.Video -> listOf("effect", "audio")
        else -> listOf("effect")
    }
    Shelf.Filters -> listOf("filter")
    Shelf.Transitions -> listOf("transition")
}

/**
 * A shelf of the catalogue: Browse, a searchable grid grouped by
 * category, where a tap applies the package to the selected clip; and
 * Adjust, the selected clip's chain links of this shelf's kinds, each
 * with sliders from its parameters that change the picture as they move.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CatalogueSheet(shelf: Shelf, catalogue: CatalogueViewModel, editor: EditorViewModel, adjustFirst: Boolean, onDismiss: () -> Unit) {
    val state by catalogue.ui.collectAsStateWithLifecycle()
    val editorUi by editor.ui.collectAsStateWithLifecycle()
    val clip = editorUi.view?.clip(editorUi.selected)
    val kinds = shelf.kinds(clip)
    var tab by remember { mutableStateOf(if (adjustFirst) 1 else 0) }
    var query by remember { mutableStateOf("") }
    var category by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) { catalogue.load() }

    val offered = state.packages.filter { it.kind in kinds }
    val shown = offered.filter { pkg ->
        (category == null || pkg.category == category) &&
            (query.isBlank() || listOf(pkg.name, pkg.category, pkg.description).any { it.contains(query.trim(), ignoreCase = true) })
    }
    val categories = offered.map { it.category }.distinct().sorted()

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SheetTitle(shelf.title)
            TabRow(selectedTabIndex = tab, containerColor = Color.Transparent) {
                Tab(selected = tab == 0, onClick = { tab = 0 }, text = { Text("Browse") })
                Tab(selected = tab == 1, onClick = { tab = 1 }, text = { Text("Adjust") }, enabled = clip != null)
            }
            if (tab == 0) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    leadingIcon = { Icon(ConcatIcons.Search, contentDescription = null, modifier = Modifier.size(20.dp)) },
                    placeholder = { Text("Search ${shelf.title.lowercase()}") },
                )
                ChoiceChips(listOf<String?>(null) + categories, category, { it ?: "All" }) { category = it }
                when {
                    !state.loaded -> SectionLabel("Loading...")
                    state.error != null -> SectionLabel(state.error ?: "")
                    shown.isEmpty() -> SectionLabel("Nothing matches.")
                }
                if (shelf != Shelf.Transitions && clip == null) SectionLabel("Select a clip to apply one.")
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(92.dp),
                    modifier = Modifier.fillMaxWidth().heightIn(max = 440.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    for ((group, packages) in shown.groupBy { it.category }.toSortedMap()) {
                        item(span = { GridItemSpan(maxLineSpan) }, key = "header-$group") {
                            Text(group, color = Palette.muted, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 6.dp))
                        }
                        items(packages, key = { it.id }) { pkg ->
                            PackageTile(pkg, enabled = clip != null || shelf == Shelf.Transitions) {
                                editor.applyPackage(pkg.kind, pkg.id, pkg.name)
                                if (pkg.params.any { it.sliderable }) tab = 1
                            }
                        }
                    }
                }
            } else {
                AdjustChain(clip, kinds, catalogue, editor)
            }
            Spacer(Modifier.navigationBarsPadding().padding(bottom = 8.dp))
        }
    }
}

@Composable
private fun PackageTile(pkg: PackageInfo, enabled: Boolean, onClick: () -> Unit) {
    val hue = (pkg.id.hashCode() and 0xff) * 360f / 256f
    Column(
        Modifier.clip(RoundedCornerShape(10.dp)).clickable(enabled = enabled, onClick = onClick).padding(4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(10.dp))
                .background(Color.hsv(hue, 0.45f, if (enabled) 0.55f else 0.3f)),
            contentAlignment = Alignment.Center,
        ) {
            Text(pkg.name.take(2), color = Color.White, fontSize = 20.sp, fontWeight = FontWeight.Bold)
        }
        Spacer(Modifier.height(4.dp))
        Text(pkg.name, color = Palette.text, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
    }
}

/** The selected clip's links of [kinds], newest first, each with its knobs. */
@Composable
private fun AdjustChain(clip: Clip?, kinds: List<String>, catalogue: CatalogueViewModel, editor: EditorViewModel) {
    if (clip == null) {
        SectionLabel("Select a clip to adjust what is on it.")
        return
    }
    // (field, index in that chain, link): indices are the chain's own, as updateClip replaces it whole.
    val links = buildList {
        if ("audio" in kinds) clip.filters.forEachIndexed { i, link -> add(Triple("filters", i, link)) }
        clip.videoEffects.forEachIndexed { i, link ->
            val kind = catalogue.find(link.id)?.kind
            val fits = when {
                clip.kind == ClipKind.Transition -> "transition" in kinds
                kind == null -> "effect" in kinds
                else -> kind in kinds
            }
            if (fits) add(Triple("videoEffects", i, link))
        }
    }.reversed()
    if (links.isEmpty()) {
        SectionLabel("Nothing applied yet: pick one under Browse.")
        return
    }
    LazyColumn(Modifier.fillMaxWidth().heightIn(max = 480.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        items(links, key = { "${clip.id}/${it.first}/${it.second}/${it.third.id}" }) { (field, index, link) ->
            LinkCard(clip, field, index, link, catalogue.find(link.id), editor, removable = clip.kind != ClipKind.Transition)
        }
    }
}

@Composable
private fun LinkCard(clip: Clip, field: String, index: Int, link: AppliedEffect, pkg: PackageInfo?, editor: EditorViewModel, removable: Boolean) {
    // The sheet's own copy of the knobs, so a drag is not pulled back by replies still on their way.
    val values = remember(clip.id, field, index, link.id) {
        mutableStateMapOf<String, Double>().apply { putAll(link.params) }
    }
    var enabled by remember(clip.id, field, index, link.id) { mutableStateOf(link.enabled) }
    fun send() {
        val out = JSONObject(link.raw.toString())
        val params = JSONObject()
        values.forEach { (key, value) -> params.put(key, value) }
        out.put("params", params).put("enabled", enabled)
        editor.setLink(clip.id, field, index, out)
    }
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Palette.raised).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(pkg?.name ?: link.id, color = Palette.text, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            Switch(checked = enabled, onCheckedChange = {
                enabled = it
                send()
            })
            if (removable) TextButton(onClick = { editor.removeLink(clip.id, field, index) }) { Text("Remove") }
        }
        val params = pkg?.params?.filter { it.sliderable }.orEmpty()
        if (params.isEmpty()) SectionLabel(pkg?.description?.ifBlank { null } ?: "Nothing to adjust.")
        for (param in params) {
            ParamControl(param, values[param.key] ?: param.default) {
                values[param.key] = it
                send()
            }
        }
    }
}

/** One knob: a slider for a number, a switch for a flag, chips for a choice. */
@Composable
private fun ParamControl(param: ParamInfo, value: Double, onChange: (Double) -> Unit) {
    when (param.type) {
        "bool" -> Row(verticalAlignment = Alignment.CenterVertically) {
            Text(param.label, color = Palette.text, fontSize = 13.sp, modifier = Modifier.weight(1f))
            Switch(checked = value != 0.0, onCheckedChange = { onChange(if (it) 1.0 else 0.0) })
        }
        "enum" -> Column {
            SectionLabel(param.label)
            val options = param.values.indices.toList()
            val current = param.values.indexOfFirst { it == value }.coerceAtLeast(0)
            ChoiceChips(options, current, { param.labels.getOrNull(it) ?: param.values[it].toString() }) { onChange(param.values[it]) }
        }
        else -> Column {
            val low = param.min.toFloat()
            val high = param.max.toFloat().coerceAtLeast(low + 1e-3f)
            val step = if (param.type == "int") param.step.coerceAtLeast(1.0) else param.step
            val steps = if (step > 0) (((high - low) / step).roundToInt() - 1).coerceIn(0, 200) else 0
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(param.label, color = Palette.text, fontSize = 13.sp, modifier = Modifier.weight(1f))
                val shown = if (param.type == "int" || step >= 1) "%.0f".format(value) else "%.2f".format(value)
                Text("$shown${param.unit}", color = Palette.accent, fontSize = 12.sp)
            }
            Slider(
                value = value.toFloat().coerceIn(low, high),
                onValueChange = { raw ->
                    var next = raw.toDouble()
                    if (step > 0) next = param.min + ((next - param.min) / step).roundToInt() * step
                    onChange(next.coerceIn(param.min, param.max))
                },
                valueRange = low..high,
                steps = steps,
            )
        }
    }
}
