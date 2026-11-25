package app.mailcrane.notify

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationChannelGroup
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.mailcrane.MainActivity
import app.mailcrane.R
import app.mailcrane.core.NewMailNotification
import app.mailcrane.widget.EXTRA_ACCOUNT_ID
import app.mailcrane.widget.EXTRA_MESSAGE_ID
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Shows new-mail notifications (M4-05): one channel per account, grouped per account, with Archive
 * and Mark read. With [hideContent] (the app lock or "Hide message content outside the app" is on)
 * they say "New message" only; on a locked screen they always do.
 *
 * Tapping one opens the message through [MainActivity], so the app lock still applies.
 */
@Singleton
class MailNotifier
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
    ) {
        private val manager = NotificationManagerCompat.from(context)

        /** Whether notifications can be shown at all (the permission on Android 13+, and not blocked). */
        fun allowed(): Boolean {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
                PackageManager.PERMISSION_GRANTED
            ) {
                return false
            }
            return manager.areNotificationsEnabled()
        }

        fun post(
            mail: List<NewMailNotification>,
            hideContent: Boolean,
        ) {
            if (mail.isEmpty() || !allowed()) return
            for ((account, messages) in mail.groupBy { it.accountId }) {
                val email = messages.first().accountEmail
                val channel = ensureChannel(account, email)
                val group = groupKey(account)
                for (m in messages) {
                    notify(notificationId(m.messageId), build(m, channel, group, hideContent))
                }
                notify(summaryId(account), summary(account, email, channel, group, hideContent))
            }
        }

        /** Removes the message's notification (after a notification action, or when it's opened). */
        fun cancel(messageId: Long) {
            manager.cancel(notificationId(messageId))
        }

        private fun build(
            m: NewMailNotification,
            channel: String,
            group: String,
            hideContent: Boolean,
        ): android.app.Notification {
            val generic = context.getString(R.string.notify_new_message)
            val title = if (hideContent) generic else m.sender.ifBlank { m.accountEmail }
            val text =
                if (hideContent) {
                    m.accountEmail
                } else {
                    m.subject.ifBlank {
                        context.getString(
                            R.string.message_no_subject,
                        )
                    }
                }
            val public =
                NotificationCompat
                    .Builder(context, channel)
                    .setSmallIcon(R.drawable.ic_mc_mail)
                    .setContentTitle(generic)
                    .setContentText(m.accountEmail)
                    .build()
            return NotificationCompat
                .Builder(context, channel)
                .setSmallIcon(R.drawable.ic_mc_mail)
                .setContentTitle(title)
                .setContentText(text)
                .setWhen(m.receivedAt * 1_000)
                .setShowWhen(true)
                .setCategory(NotificationCompat.CATEGORY_EMAIL)
                .setGroup(group)
                .setAutoCancel(true)
                .setOnlyAlertOnce(true)
                .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                .setPublicVersion(public)
                .setContentIntent(openIntent(m))
                .addAction(0, context.getString(R.string.notify_action_archive), actionIntent(m, ACTION_ARCHIVE))
                .addAction(0, context.getString(R.string.notify_action_mark_read), actionIntent(m, ACTION_MARK_READ))
                .build()
        }

        private fun summary(
            account: Long,
            email: String,
            channel: String,
            group: String,
            hideContent: Boolean,
        ): android.app.Notification =
            NotificationCompat
                .Builder(context, channel)
                .setSmallIcon(R.drawable.ic_mc_mail)
                .setContentTitle(context.getString(R.string.notify_new_message))
                .setContentText(email)
                .setSubText(if (hideContent) null else email)
                .setGroup(group)
                .setGroupSummary(true)
                .setAutoCancel(true)
                .setOnlyAlertOnce(true)
                .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                .setContentIntent(openAppIntent(account))
                .build()

        private fun openIntent(m: NewMailNotification): PendingIntent {
            val intent =
                Intent(context, MainActivity::class.java)
                    .putExtra(EXTRA_ACCOUNT_ID, m.accountId)
                    .putExtra(EXTRA_MESSAGE_ID, m.messageId)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            return PendingIntent.getActivity(
                context,
                notificationId(m.messageId),
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }

        private fun openAppIntent(account: Long): PendingIntent {
            val intent =
                Intent(context, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            return PendingIntent.getActivity(
                context,
                summaryId(account),
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }

        private fun actionIntent(
            m: NewMailNotification,
            action: String,
        ): PendingIntent {
            val intent =
                Intent(context, NotificationActionReceiver::class.java)
                    .setAction(action)
                    .putExtra(EXTRA_MESSAGE_ID, m.messageId)
            val request = notificationId(m.messageId) * 2 + if (action == ACTION_ARCHIVE) 0 else 1
            return PendingIntent.getBroadcast(
                context,
                request,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }

        private fun ensureChannel(
            account: Long,
            email: String,
        ): String {
            val id = "mail-$account"
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val system = context.getSystemService(NotificationManager::class.java)
                system?.createNotificationChannelGroup(
                    NotificationChannelGroup(GROUP_CHANNELS, context.getString(R.string.notify_channel_group)),
                )
                val channel =
                    NotificationChannel(id, email, NotificationManager.IMPORTANCE_DEFAULT).apply {
                        description = context.getString(R.string.notify_channel_description)
                        group = GROUP_CHANNELS
                        lockscreenVisibility = android.app.Notification.VISIBILITY_PRIVATE
                    }
                system?.createNotificationChannel(channel)
            }
            return id
        }

        @Suppress("MissingPermission") // allowed() is checked before every post
        private fun notify(
            id: Int,
            notification: android.app.Notification,
        ) {
            try {
                manager.notify(id, notification)
            } catch (_: SecurityException) {
                // The permission was revoked between the check and now.
            }
        }

        companion object {
            const val ACTION_ARCHIVE = "app.mailcrane.notify.ARCHIVE"
            const val ACTION_MARK_READ = "app.mailcrane.notify.MARK_READ"
            private const val GROUP_CHANNELS = "accounts"

            // Message notifications use the message id; summaries live below zero.
            fun notificationId(messageId: Long): Int = (messageId and 0x3fffffff).toInt()

            private fun summaryId(account: Long): Int = -1 - (account and 0x3fffffff).toInt()

            private fun groupKey(account: Long) = "app.mailcrane.mail.$account"
        }
    }
