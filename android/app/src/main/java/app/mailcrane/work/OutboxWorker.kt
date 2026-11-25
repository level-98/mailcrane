package app.mailcrane.work

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.mailcrane.core.OutboxFailure
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.databasePath
import app.mailcrane.core.outboxAccounts
import app.mailcrane.core.sendOutbox
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.TimeUnit

/**
 * Sends the outbox when there is a network, even with the app in the background (M2-06). Retried by
 * WorkManager with back-off while messages are left.
 */
class OutboxWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        // The Application's start-up recovery must finish first (see OutboxSender.start).
        withTimeoutOrNull(READY_TIMEOUT_MS) { OutboxSender.ready.await() } ?: return Result.retry()
        val dbPath = databasePath(applicationContext)
        val accounts =
            try {
                outboxAccounts(dbPath)
            } catch (_: OutboxFailure) {
                return Result.retry()
            }
        var left = 0u
        for (account in accounts) {
            left +=
                try {
                    sendOutbox(dbPath, account).remaining
                } catch (_: SyncFailure) {
                    1u
                }
        }
        return if (left == 0u) Result.success() else Result.retry()
    }

    companion object {
        private const val NAME = "outbox"
        private const val READY_TIMEOUT_MS = 60_000L

        /**
         * Runs the job at [sendAt] (Unix seconds) once there is a network, for a scheduled message. Not an
         * exact alarm (that needs a special permission): Android may run it a little late.
         */
        fun scheduleAt(
            context: Context,
            sendAt: Long,
        ) {
            val delayMs = (sendAt * 1_000 - System.currentTimeMillis()).coerceAtLeast(0)
            val request =
                OneTimeWorkRequestBuilder<OutboxWorker>()
                    .setInitialDelay(delayMs, TimeUnit.MILLISECONDS)
                    .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                    .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                    .build()
            WorkManager.getInstance(context).enqueueUniqueWork("$NAME-at-$sendAt", ExistingWorkPolicy.KEEP, request)
        }

        /** Runs the job once there is a network; does nothing if it's already waiting. */
        fun schedule(context: Context) {
            val request =
                OneTimeWorkRequestBuilder<OutboxWorker>()
                    .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                    .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                    .build()
            WorkManager.getInstance(context).enqueueUniqueWork(NAME, ExistingWorkPolicy.KEEP, request)
        }
    }
}
