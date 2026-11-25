package app.mailcrane.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.mailcrane.MainActivity
import app.mailcrane.R
import app.mailcrane.core.LiveUpdatesEngine
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * "Always-on sync" (M4-05): keeps every account's live updates (IMAP IDLE, or polling) running
 * with the app in the background, so new mail is notified within a minute. Opt-in, because it
 * keeps a connection open and shows a permanent, quiet notification as Android requires.
 *
 * The connections themselves are the same ones the app uses while open; the Application
 * leaves them running when the app goes to the background while this service runs.
 */
@AndroidEntryPoint
class AlwaysOnSyncService : Service() {
    @Inject lateinit var engine: LiveUpdatesEngine

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        val type =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            } else {
                0
            }
        try {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, ongoing(), type)
        } catch (_: RuntimeException) {
            // Android refused (started from the background where it isn't allowed): the periodic
            // check still runs, and the next app start tries again.
            stopSelf()
            return START_NOT_STICKY
        }
        running = true
        scope.launch {
            for (id in engine.accountIds()) engine.start(id)
        }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        scope.cancel()
        super.onDestroy()
    }

    private fun ongoing(): Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel =
                NotificationChannel(
                    CHANNEL,
                    getString(R.string.always_on_channel),
                    NotificationManager.IMPORTANCE_MIN,
                ).apply { setShowBadge(false) }
            getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
        }
        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
        return NotificationCompat
            .Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_mc_mail)
            .setContentTitle(getString(R.string.always_on_notification))
            .setOngoing(true)
            .setSilent(true)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .setContentIntent(open)
            .build()
    }

    companion object {
        private const val CHANNEL = "always-on"
        private const val NOTIFICATION_ID = 1

        /** Whether the service is running now, so live updates are kept on in the background. */
        @Volatile
        var running: Boolean = false
            private set

        fun start(context: Context) {
            try {
                ContextCompat.startForegroundService(context, Intent(context, AlwaysOnSyncService::class.java))
            } catch (_: RuntimeException) {
                // Not allowed from the background right now; tried again on the next app start.
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, AlwaysOnSyncService::class.java))
        }
    }
}
