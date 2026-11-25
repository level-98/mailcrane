package app.mailcrane.work

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.mailcrane.MainActivity
import app.mailcrane.R
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.CoreEventListener
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.SnoozeFailure
import app.mailcrane.core.SyncEngine
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.WokenMessage
import app.mailcrane.core.databasePath
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Brings snoozed messages back (M4-06) while the app runs: [MainActivity] calls [wake] when it opens
 * and every minute while it's shown. [SnoozeWorker] does the same at the next wake time when the app
 * is in the background or closed.
 */
@Singleton
class SnoozeWaker
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
        private val core: MailboxCore,
    ) {
        /** Wakes what's due, notifies, and schedules the background job for the next wake time. */
        suspend fun wake() {
            try {
                SnoozeNotifier.notify(context, core.wakeDueSnoozes())
                SnoozeWorker.scheduleAt(context, core.nextSnoozeWake())
            } catch (_: SnoozeFailure) {
                // The database failed; the next tick tries again.
            }
        }
    }

/** Wakes due snoozes in the background and moves the messages back on the server. */
class SnoozeWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result =
        withContext(Dispatchers.IO) {
            val engine = SyncEngine(databasePath(applicationContext), NoSnoozeEvents)
            try {
                val woken = engine.wakeDueSnoozes()
                SnoozeNotifier.notify(applicationContext, woken)
                var left = 0u
                for (account in woken.map { it.accountId }.distinct()) {
                    left +=
                        try {
                            engine.replayActions(account)
                        } catch (_: SyncFailure) {
                            1u
                        }
                }
                scheduleAt(applicationContext, engine.nextSnoozeWake())
                // Offline: the messages are already back on this device; try the server move later.
                if (left == 0u) Result.success() else Result.retry()
            } catch (_: SnoozeFailure) {
                Result.retry()
            }
        }

    companion object {
        private const val NAME = "snooze-wake"

        /**
         * Runs the job at [wakeAt] (Unix seconds), replacing an earlier schedule; null cancels it. Not an
         * exact alarm (that needs a special permission): Android may run it a little late.
         */
        fun scheduleAt(
            context: Context,
            wakeAt: Long?,
        ) {
            val work = WorkManager.getInstance(context)
            if (wakeAt == null) {
                work.cancelUniqueWork(NAME)
                return
            }
            val delayMs = (wakeAt * 1_000 - System.currentTimeMillis()).coerceAtLeast(0)
            val request =
                OneTimeWorkRequestBuilder<SnoozeWorker>()
                    .setInitialDelay(delayMs, TimeUnit.MILLISECONDS)
                    .build()
            work.enqueueUniqueWork(NAME, ExistingWorkPolicy.REPLACE, request)
        }
    }
}

private object NoSnoozeEvents : CoreEventListener {
    override fun onEvent(event: CoreEvent) {}
}

/** Posts "Back from snooze" notifications, when the user allowed notifications. */
object SnoozeNotifier {
    private const val CHANNEL = "snoozed"

    fun notify(
        context: Context,
        woken: List<WokenMessage>,
    ) {
        if (woken.isEmpty() || !allowed(context)) return
        val manager = NotificationManagerCompat.from(context)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel =
                NotificationChannel(
                    CHANNEL,
                    context.getString(R.string.snooze_channel_name),
                    NotificationManager.IMPORTANCE_DEFAULT,
                ).apply { description = context.getString(R.string.snooze_channel_description) }
            context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
        }
        val open =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        for (message in woken) {
            val subject = message.subject.ifBlank { context.getString(R.string.snooze_woken_no_subject) }
            val notification =
                NotificationCompat
                    .Builder(context, CHANNEL)
                    .setSmallIcon(R.drawable.ic_mc_snooze)
                    .setContentTitle(context.getString(R.string.snooze_woken_title))
                    .setContentText(if (message.from.isBlank()) subject else "${message.from}: $subject")
                    .setContentIntent(open)
                    .setAutoCancel(true)
                    // Message text stays off the lock screen.
                    .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                    .build()
            try {
                manager.notify(id(message), notification)
            } catch (_: SecurityException) {
                return
            }
        }
    }

    private fun allowed(context: Context): Boolean =
        NotificationManagerCompat.from(context).areNotificationsEnabled() &&
            (
                Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                    ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
                    PackageManager.PERMISSION_GRANTED
            )

    private fun id(message: WokenMessage): Int = (message.messageId ?: message.subject.hashCode().toLong()).toInt()
}
