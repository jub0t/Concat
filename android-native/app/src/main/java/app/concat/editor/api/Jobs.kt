// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.api

import org.json.JSONObject

/**
 * One job's events out of the API's stream: those named `prefix.*` whose
 * `job` is the one [start] named. An engine may raise the first event
 * before the reply naming the job lands, so events that come while no job
 * is known are held and replayed once it is.
 *
 * Fed and started from one thread (the main one, in a ViewModel).
 */
class JobTracker(private val prefix: String, private val handle: (JSONObject) -> Unit) {
    var job: String? = null
        private set
    private var waiting = false
    private val early = ArrayList<JSONObject>()

    /** The request is on its way: hold what comes until its job is known. */
    fun expect() {
        job = null
        waiting = true
        early.clear()
    }

    fun start(job: String) {
        this.job = job
        waiting = false
        val held = early.filter { it.optString("job") == job }
        early.clear()
        held.forEach(handle)
    }

    fun stop() {
        job = null
        waiting = false
        early.clear()
    }

    fun offer(event: JSONObject) {
        if (!event.optString("event").startsWith("$prefix.")) return
        val current = job
        when {
            current != null -> if (event.optString("job") == current) handle(event)
            waiting -> if (early.size < 256) early += event
        }
    }
}

/** An event's `error.message`, for a sentence on screen. */
fun JSONObject.errorMessage(): String = optJSONObject("error")?.optString("message")?.ifBlank { null } ?: "It did not work."

/** Whether an event's error is the one a cancel ends a job with. */
fun JSONObject.wasCancelled(): Boolean = optJSONObject("error")?.optString("code") == "cancelled"
