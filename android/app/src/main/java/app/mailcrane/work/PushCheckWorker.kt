package app.mailcrane.work

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequest
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.mailcrane.R
import app.mailcrane.core.SyncFailure
import dagger.hilt.android.EntryPointAccessors

/**
 * Checks every inbox once, right after a push wake-up (UnifiedPush, M4-05). Expedited, so it runs
 * at once even in Doze, within Android's quota; past the quota it runs as ordinary work.
 *
 * Wake-ups that arrive while a check is queued are merged into it, so a flood of push messages
 * costs one check at a time.
 */
class PushCheckWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val deps = EntryPointAccessors.fromApplication(applicationContext, MailCheckWorker.Deps::class.java)
        val mail =
            try {
                deps.core().backgroundSync(DEADLINE_SECONDS)
            } catch (_: SyncFailure) {
                return Result.success() // the periodic check is still there
            }
        deps.checker().show(mail)
        return Result.success()
    }

    /** Before Android 12, expedited work runs as a short foreground service and needs this. */
    override suspend fun getForegroundInfo(): ForegroundInfo = ForegroundInfo(NOTIFICATION_ID, notification())

    private fun notification(): Notification {
        val context = applicationContext
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel =
                NotificationChannel(
                    CHANNEL,
                    context.getString(R.string.push_check_channel),
                    NotificationManager.IMPORTANCE_MIN,
                )
            context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
        }
        return NotificationCompat
            .Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_mc_mail)
            .setContentTitle(context.getString(R.string.push_check_notification))
            .setSilent(true)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .build()
    }

    companion object {
        /** The unique work name: one check queued at a time. */
        const val NAME = "push-check"
        private const val CHANNEL = "push-check"
        private const val NOTIFICATION_ID = 2

        // Expedited work should finish quickly; one inbox sync per account fits.
        private const val DEADLINE_SECONDS = 60

        /** The work request for one check. */
        fun request(): OneTimeWorkRequest =
            OneTimeWorkRequestBuilder<PushCheckWorker>()
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()

        fun enqueue(context: Context) {
            // KEEP: a check already queued covers this wake-up too.
            WorkManager.getInstance(context).enqueueUniqueWork(NAME, ExistingWorkPolicy.KEEP, request())
        }
    }
}
