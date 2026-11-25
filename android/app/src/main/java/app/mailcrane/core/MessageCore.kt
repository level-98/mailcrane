package app.mailcrane.core

/**
 * Reading one message through the core. An interface so the message screen can be tested with a
 * fake. Suspending calls run off the main thread and throw [SyncFailure] when they fail.
 */
interface MessageCore {
    /**
     * The message, ready to show. HTML is already sanitised and wrapped in a locked-down document;
     * remote images are left in only when [allowRemote]. Reads only the database.
     */
    suspend fun getMessage(
        messageId: Long,
        allowRemote: Boolean,
    ): MessageDetail

    /** Downloads the body once, for offline reading. Doesn't mark the message as read. */
    suspend fun fetchBody(messageId: Long)

    /** Marks the message as read, at once on this device and then on the server. */
    suspend fun markRead(messageId: Long)

    /**
     * Applies [action] to the message: the change shows at once and is sent to the server in the
     * background, retried while the app runs. Throws [ActionFailure] when it can't be applied (for
     * example [ActionFailure.NoArchiveFolder]); nothing is changed then.
     */
    suspend fun applyAction(
        messageId: Long,
        action: MessageAction,
    )

    /**
     * Snoozes the message until [wakeAt] (Unix seconds): it leaves its folder at once and moves to the
     * Snoozed folder on the server in the background. A message already snoozed gets the new time.
     * @throws SnoozeFailure
     */
    suspend fun snooze(
        messageId: Long,
        wakeAt: Long,
    )

    /** Brings a snoozed message back now, without marking it unread. @throws SnoozeFailure */
    suspend fun unsnooze(messageId: Long)

    /** The message's snooze, or null if it isn't snoozed. @throws SnoozeFailure */
    suspend fun snoozeInfo(messageId: Long): SnoozeInfo?

    /**
     * The inline image with Content-ID [contentId] from this message's stored body, or null.
     * Blocking: called on the web view's background thread.
     */
    fun inlinePart(
        messageId: Long,
        contentId: String,
    ): InlinePart?
}
