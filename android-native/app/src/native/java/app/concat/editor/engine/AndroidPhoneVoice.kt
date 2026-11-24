// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.engine

import android.content.Context
import android.os.Bundle
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.util.Log
import java.io.File
import java.util.UUID
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import uniffi.concat_mobile.PhoneVoice
import uniffi.concat_mobile.setPhoneVoice

/**
 * The phone's own text-to-speech as read aloud's "Phone voice". The engine
 * calls [say] on one of its worker threads, a piece of the text at a time,
 * and joins the WAVs it writes. Handed to the engine only once the phone's
 * engine has started, so a phone without one never lists a voice that
 * cannot read.
 */
internal class AndroidPhoneVoice(context: Context) : PhoneVoice {
    private val waiting = ConcurrentHashMap<String, CompletableFuture<String?>>()

    /** Held while the rate is set and the piece queued: a request takes the rate in force when it is queued. */
    private val queueing = Any()

    private val tts: TextToSpeech = TextToSpeech(context.applicationContext) { status ->
        if (status == TextToSpeech.SUCCESS) {
            setPhoneVoice(this)
            Log.i(TAG, "the phone's voice is ready: ${engineName()}")
        } else {
            Log.w(TAG, "the phone has no text-to-speech that starts ($status); read aloud offers no phone voice")
        }
    }

    init {
        tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String) {}

            override fun onDone(utteranceId: String) {
                waiting[utteranceId]?.complete(null)
            }

            override fun onError(utteranceId: String, errorCode: Int) {
                waiting[utteranceId]?.complete("the phone's voice could not read this (error $errorCode)")
            }

            @Deprecated("Superseded by onError(String, Int)")
            override fun onError(utteranceId: String) {
                waiting[utteranceId]?.complete("the phone's voice could not read this")
            }
        })
    }

    override fun say(text: String, path: String, rate: Float): String? {
        val id = UUID.randomUUID().toString()
        val done = CompletableFuture<String?>()
        waiting[id] = done
        try {
            val queued = synchronized(queueing) {
                tts.setSpeechRate(rate)
                tts.synthesizeToFile(text, Bundle(), File(path), id)
            }
            if (queued != TextToSpeech.SUCCESS) return "the phone's voice would not take the text"
            return try {
                done.get(PIECE_SECONDS, TimeUnit.SECONDS)
            } catch (_: TimeoutException) {
                "the phone's voice took over $PIECE_SECONDS seconds on one piece"
            }
        } finally {
            waiting.remove(id)
        }
    }

    private fun engineName(): String = tts.defaultEngine ?: "unnamed engine"

    private companion object {
        const val TAG = "concat-speech"

        /** A piece is at most ~3,900 characters: minutes of speech, written faster than it is read. */
        const val PIECE_SECONDS = 180L
    }
}
