package app.mailcrane.work

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.mailcrane.core.NotificationsCore
import app.mailcrane.core.SyncFailure
import app.mailcrane.notify.NewMailChecker
import app.mailcrane.sync.AlwaysOnSyncService
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import java.util.concurrent.TimeUnit

/**
 * Checks every inbox for new mail about every 15 minutes (Android's shortest period) (M4-05).
 * Android decides the exact time and may wait longer to save battery.
 *
 * With always-on sync it is a watchdog: while the service runs it does nothing. If the phone
 * stopped the service, starting this worker's process starts it again where Android allows (the
 * app is exempt from battery optimisation), and this check covers the gap either way.
 */
class MailCheckWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    @EntryPoint
    @InstallIn(SingletonComponent::class)
    interface Deps {
        fun core(): NotificationsCore

        fun checker(): NewMailChecker
    }

    override suspend fun doWork(): Result {
        if (AlwaysOnSyncService.running) return Result.success()
        val deps = EntryPointAccessors.fromApplication(applicationContext, Deps::class.java)
        val mail =
            try {
                deps.core().backgroundSync(DEADLINE_SECONDS)
            } catch (_: SyncFailure) {
                return Result.success() // the next period tries again
            }
        deps.checker().show(mail)
        return Result.success()
    }

    companion object {
        private const val NAME = "mail-check"

        // Well inside WorkManager's 10-minute limit.
        private const val DEADLINE_SECONDS = 8 * 60

        fun schedule(context: Context) {
            val request =
                PeriodicWorkRequestBuilder<MailCheckWorker>(15, TimeUnit.MINUTES)
                    .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                    .build()
            WorkManager
                .getInstance(context)
                .enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        fun cancel(context: Context) {
            WorkManager.getInstance(context).cancelUniqueWork(NAME)
        }
    }
}
