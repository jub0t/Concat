// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.export

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.concat.editor.spike.MainActivity
import app.concat.editor.spike.concat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Keeps the process in the foreground while an export renders, so the
 * screen going off or the user leaving the app does not stop it, and shows
 * its progress in a notification with a Cancel action. The render itself
 * runs in the engine; this only watches [ExportManager.state].
 * `mediaProcessing` on API 35, `dataSync` before it.
 */
class ExportService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var watching: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val exports = concat.exports
        if (intent?.action == ACTION_CANCEL) {
            exports.cancel()
            return START_NOT_STICKY
        }
        channel()
        val running = exports.state.value as? ExportState.Running
        startInForeground(progress(running?.title ?: "Export", running?.stage ?: "starting", running?.fraction ?: 0f))
        if (watching == null) {
            watching = scope.launch {
                exports.state.collect { state ->
                    when (state) {
                        is ExportState.Running -> notify(progress(state.title, state.stage, state.fraction))
                        is ExportState.Done -> end(finished(state.title, state.note ?: "Saved to Movies/Concat"))
                        is ExportState.Failed -> end(finished(state.title, if (state.cancelled) "Cancelled" else state.message))
                        ExportState.Idle -> end(null)
                    }
                }
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun startInForeground(notification: Notification) {
        val type = when {
            Build.VERSION.SDK_INT >= 35 -> ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q -> ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            else -> 0
        }
        ServiceCompat.startForeground(this, PROGRESS_ID, notification, type)
    }

    private fun end(notification: Notification?) {
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        if (notification != null) notify(notification, DONE_ID)
        watching?.cancel()
        watching = null
        stopSelf()
    }

    private fun notify(notification: Notification, id: Int = PROGRESS_ID) {
        val allowed = Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        if (allowed) NotificationManagerCompat.from(this).notify(id, notification)
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this, 0,
        Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun progress(title: String, stage: String, fraction: Float): Notification {
        val cancel = PendingIntent.getService(
            this, 1,
            Intent(this, ExportService::class.java).setAction(ACTION_CANCEL),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle("Exporting $title")
            .setContentText("${(fraction * 100).toInt()}% - $stage")
            .setProgress(1000, (fraction * 1000).toInt(), fraction <= 0f)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(openApp())
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Cancel", cancel)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    private fun finished(title: String, text: String): Notification =
        NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle(title)
            .setContentText(text)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()

    private fun channel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, "Exports", NotificationManager.IMPORTANCE_LOW),
            )
        }
    }

    companion object {
        const val ACTION_CANCEL = "app.concat.editor.spike.export.CANCEL"
        private const val CHANNEL = "exports"
        private const val PROGRESS_ID = 1
        private const val DONE_ID = 2
    }
}
