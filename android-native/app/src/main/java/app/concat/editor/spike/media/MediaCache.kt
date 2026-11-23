// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.media

import android.graphics.BitmapFactory
import android.util.Log
import android.util.LruCache
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import app.concat.editor.spike.api.ConcatApi
import app.concat.editor.spike.api.Peaks
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext

/** One strip as asked for: the file, the stretch of it, how many pictures and how tall. */
data class StripKey(val file: String, val from: Double, val to: Double, val count: Int, val height: Int) {
    /** The same strip at any count: what stands in while the right count loads. */
    val range get() = RangeKey(file, from, to, height)
}

data class RangeKey(val file: String, val from: Double, val to: Double, val height: Int)

/**
 * The timeline's pictures and sound outlines (`media.filmstrip`,
 * `media.peaks`), fetched once and kept in memory, least recently used
 * first out. Reads come from drawing code on the main thread and never
 * block: a miss starts the request and returns what it has, and
 * [version] changes when something lands so whatever drew it draws again.
 */
class MediaCache(private val api: ConcatApi, private val scope: CoroutineScope) {
    /** Read by whatever draws from the cache, so a load redraws it. */
    var version by mutableIntStateOf(0)
        private set

    private val strips = object : LruCache<StripKey, ImageBitmap>(STRIP_BYTES) {
        override fun sizeOf(key: StripKey, value: ImageBitmap) = value.width * value.height * 4
    }
    private val nearest = object : LruCache<RangeKey, ImageBitmap>(STRIP_BYTES / 4) {
        override fun sizeOf(key: RangeKey, value: ImageBitmap) = value.width * value.height * 4
    }
    private val peaks = LruCache<String, Peaks>(48)
    private val pending = HashSet<Any>()
    private val refused = HashSet<Any>()
    private val gate = Semaphore(2)

    /** The strip for [key], or the nearest one held for the same stretch, or null while it loads. */
    fun strip(key: StripKey): ImageBitmap? {
        strips.get(key)?.let { return it }
        if (key !in pending && key !in refused) {
            pending += key
            scope.launch(Dispatchers.Main) {
                try {
                    val image = gate.withPermit {
                        val bytes = api.mediaFilmstrip(key.file, key.from, key.to, key.count, key.height)
                        withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }
                    }
                    if (image != null) {
                        strips.put(key, image)
                        nearest.put(key.range, image)
                        version++
                    } else {
                        refused += key
                    }
                } catch (error: Exception) {
                    // An engine without the method, a file that will not decode:
                    // the clip keeps its plain colour.
                    Log.w(TAG, "no filmstrip for ${key.file}: ${error.message}")
                    refused += key
                } finally {
                    pending -= key
                }
            }
        }
        return nearest.get(key.range)
    }

    /** [file]'s sound outline, or null while it loads. */
    fun peaks(projectPath: String?, file: String): Peaks? {
        peaks.get(file)?.let { return it }
        val key = "peaks:$file"
        if (key !in pending && key !in refused) {
            pending += key
            scope.launch(Dispatchers.Main) {
                try {
                    val outline = gate.withPermit { api.mediaPeaks(projectPath, file, PER_SECOND) }
                    peaks.put(file, outline)
                    version++
                } catch (error: Exception) {
                    Log.w(TAG, "no peaks for $file: ${error.message}")
                    refused += key
                } finally {
                    pending -= key
                }
            }
        }
        return null
    }

    companion object {
        private const val TAG = "concat-media"
        private const val STRIP_BYTES = 24 * 1024 * 1024
        const val PER_SECOND = 20

        /** The strip counts asked for: a few sizes, so zooming reuses strips rather than asking anew at every step. */
        private val COUNTS = intArrayOf(1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 60)

        fun bucket(wanted: Int): Int = COUNTS.firstOrNull { it >= wanted } ?: COUNTS.last()
    }
}
