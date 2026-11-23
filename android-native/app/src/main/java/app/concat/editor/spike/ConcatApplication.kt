// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike

import android.app.Application
import android.content.Context
import app.concat.editor.spike.api.ConcatApi
import app.concat.editor.spike.engine.EngineFactory
import app.concat.editor.spike.export.ExportManager
import app.concat.editor.spike.media.MediaCache
import app.concat.editor.spike.models.ModelStore
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** One editor per process: the API, and the export that outlives any screen. */
class ConcatApplication : Application() {
    /** Work that belongs to the process, not to a screen: an export, a save on the way out. */
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val api: ConcatApi by lazy { ConcatApi(EngineFactory.create(this)) }
    val exports: ExportManager by lazy { ExportManager(this, api, scope) }
    val models: ModelStore by lazy { ModelStore(api, scope) }
    /** The timeline's filmstrips and waveforms, cached in memory. */
    val media: MediaCache by lazy { MediaCache(api, scope) }

    /** Where new projects are made. */
    val projectsDir: File get() = File(filesDir, "projects").apply { mkdirs() }

    /** Where picked files are copied: the engine reads paths, not content URIs. */
    val mediaDir: File get() = File(filesDir, "media").apply { mkdirs() }
}

val Context.concat: ConcatApplication get() = applicationContext as ConcatApplication
