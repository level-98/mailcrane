package app.mailcrane.work

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.ListenableWorker
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequest
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequest
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import app.mailcrane.R
import app.mailcrane.core.JmapPushCore
import app.mailcrane.push.PushStatus
import app.mailcrane.push.UnifiedPushState
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import java.util.concurrent.TimeUnit

/** What [JmapPushWorker] does. */
enum class JmapPushOp {
    /** Register (or refresh) the current endpoint with every JMAP account. */
    REGISTER,

    /** Answer a server's verification push. */
    VERIFY,

    /** Remove every subscription: push wake-ups were turned off. */
    UNREGISTER,
}

/**
 * Runs one [op] against [core] (M4-27). [status] is the push state now; [body] the push for
 * [JmapPushOp.VERIFY]. Separate from the worker so it can be tested without WorkManager or Hilt.
 */
suspend fun runJmapPush(
    op: JmapPushOp,
    status: PushStatus,
    body: ByteArray?,
    core: JmapPushCore,
    attempt: Int,
): ListenableWorker.Result =
    when (op) {
        JmapPushOp.REGISTER -> {
            // Only a live endpoint is registered; turning push off unregisters separately.
            if (status is PushStatus.Ready) {
                core.registerPush(status.endpoint, status.keys?.p256dh, status.keys?.auth)
            }
            ListenableWorker.Result.success()
        }
        JmapPushOp.VERIFY -> {
            when {
                body == null || status !is PushStatus.Ready -> ListenableWorker.Result.success()
                core.answerPush(body) -> ListenableWorker.Result.success()
                // The server waits a while for the code; a network blip deserves another try.
                attempt < MAX_VERIFY_ATTEMPTS -> ListenableWorker.Result.retry()
                // Given up: the next registration makes the subscription again.
                else -> ListenableWorker.Result.success()
            }
        }
        JmapPushOp.UNREGISTER -> {
            core.unregisterPush()
            ListenableWorker.Result.success()
        }
    }

private const val MAX_VERIFY_ATTEMPTS = 3

/**
 * JMAP push subscriptions in the background (M4-27): registering the UnifiedPush endpoint with
 * each JMAP account when the distributor gives one and once a day (to refresh them before they
 * expire, and to pick up accounts added since), answering verification pushes at once
 * (expedited), and removing them when push wake-ups are turned off.
 */
class JmapPushWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    @EntryPoint
    @InstallIn(SingletonComponent::class)
    interface Deps {
        fun jmapPush(): JmapPushCore
    }

    override suspend fun doWork(): Result {
        val op = inputData.getString(KEY_OP)?.let { name -> JmapPushOp.entries.firstOrNull { it.name == name } }
        if (op == null) return Result.success()
        val core = EntryPointAccessors.fromApplication(applicationContext, Deps::class.java).jmapPush()
        val status = UnifiedPushState.get(applicationContext).status.value
        return runJmapPush(op, status, inputData.getByteArray(KEY_BODY), core, runAttemptCount)
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
            .setContentTitle(context.getString(R.string.push_setup_notification))
            .setSilent(true)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .build()
    }

    companion object {
        private const val KEY_OP = "op"
        private const val KEY_BODY = "body"
        private const val CHANNEL = "push-check"
        private const val NOTIFICATION_ID = 3

        /** Unique work names: one of each queued at a time. */
        const val NAME_REGISTER = "jmap-push-register"
        const val NAME_REFRESH = "jmap-push-refresh"
        const val NAME_VERIFY = "jmap-push-verify"
        const val NAME_UNREGISTER = "jmap-push-unregister"

        /** Bigger pushes can't be verifications (they're under 200 bytes) or fit in work data. */
        const val MAX_VERIFY_BYTES = 4 * 1024

        private val network = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()

        private fun data(
            op: JmapPushOp,
            body: ByteArray? = null,
        ): Data {
            val builder = Data.Builder().putString(KEY_OP, op.name)
            if (body != null) builder.putByteArray(KEY_BODY, body)
            return builder.build()
        }

        fun registerRequest(): OneTimeWorkRequest =
            OneTimeWorkRequestBuilder<JmapPushWorker>()
                .setInputData(data(JmapPushOp.REGISTER))
                .setConstraints(network)
                .build()

        fun refreshRequest(): PeriodicWorkRequest =
            PeriodicWorkRequestBuilder<JmapPushWorker>(1, TimeUnit.DAYS)
                .setInputData(data(JmapPushOp.REGISTER))
                .setConstraints(network)
                .build()

        /** The request for answering [body], or null if it's too big to be a verification. */
        fun verifyRequest(body: ByteArray): OneTimeWorkRequest? {
            if (body.size > MAX_VERIFY_BYTES) return null
            return OneTimeWorkRequestBuilder<JmapPushWorker>()
                .setInputData(data(JmapPushOp.VERIFY, body))
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .setConstraints(network)
                .build()
        }

        fun unregisterRequest(): OneTimeWorkRequest =
            OneTimeWorkRequestBuilder<JmapPushWorker>()
                .setInputData(data(JmapPushOp.UNREGISTER))
                .setConstraints(network)
                .build()

        /** A new endpoint: register it now, and keep it fresh daily. */
        fun onNewEndpoint(context: Context) {
            val work = WorkManager.getInstance(context)
            work.enqueueUniqueWork(NAME_REGISTER, ExistingWorkPolicy.REPLACE, registerRequest())
            work.enqueueUniquePeriodicWork(NAME_REFRESH, ExistingPeriodicWorkPolicy.KEEP, refreshRequest())
        }

        /** A verification push: answer it soon. Each gets its own work, so none is dropped. */
        fun verify(
            context: Context,
            body: ByteArray,
        ) {
            val request = verifyRequest(body) ?: return
            WorkManager
                .getInstance(
                    context,
                ).enqueueUniqueWork(NAME_VERIFY, ExistingWorkPolicy.APPEND_OR_REPLACE, request)
        }

        /** Push wake-ups were turned off: stop refreshing and remove the subscriptions. */
        fun turnedOff(context: Context) {
            val work = WorkManager.getInstance(context)
            work.cancelUniqueWork(NAME_REFRESH)
            work.cancelUniqueWork(NAME_REGISTER)
            work.enqueueUniqueWork(NAME_UNREGISTER, ExistingWorkPolicy.REPLACE, unregisterRequest())
        }
    }
}
