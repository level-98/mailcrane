package app.mailcrane.core

/**
 * New-mail notifications from the core (M4-05). The core decides which messages deserve one and
 * remembers them, so each message is returned at most once, ever.
 */
interface NotificationsCore {
    /** New mail from what is stored now; no network. */
    suspend fun takeNewMail(): List<NewMailNotification>

    /** Syncs every inbox (stopping before [deadlineSeconds]), then returns the new mail. */
    suspend fun backgroundSync(deadlineSeconds: Int): List<NewMailNotification>

    /** A notification button: archive or mark read, like the same action in the app. */
    suspend fun applyAction(
        messageId: Long,
        action: MessageAction,
    )
}
