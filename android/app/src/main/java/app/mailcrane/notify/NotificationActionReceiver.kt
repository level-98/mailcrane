package app.mailcrane.notify

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.mailcrane.core.MessageAction
import app.mailcrane.core.NotificationsCore
import app.mailcrane.widget.EXTRA_MESSAGE_ID
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * The Archive and Mark read buttons on a notification (M4-05). They work like the same actions in
 * the app: applied locally at once and queued for the server. Not exported: only this app's own
 * notifications can send it anything.
 */
@AndroidEntryPoint
class NotificationActionReceiver : BroadcastReceiver() {
    @Inject lateinit var core: NotificationsCore

    @Inject lateinit var notifier: MailNotifier

    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val messageId = intent.getLongExtra(EXTRA_MESSAGE_ID, 0L)
        val action =
            when (intent.action) {
                MailNotifier.ACTION_ARCHIVE -> MessageAction.Archive
                MailNotifier.ACTION_MARK_READ -> MessageAction.MarkRead
                else -> return
            }
        if (messageId <= 0) return
        notifier.cancel(messageId)
        val pending = goAsync()
        scope.launch {
            try {
                core.applyAction(messageId, action)
            } catch (_: Exception) {
                // The message is gone or the store is busy: nothing to undo; the next sync shows the truth.
            } finally {
                pending.finish()
            }
        }
    }

    private companion object {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
