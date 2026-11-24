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

    /** Marks the message as read (on this device only until M1-14 syncs it to the server). */
    suspend fun markRead(messageId: Long)

    /**
     * The inline image with Content-ID [contentId] from this message's stored body, or null.
     * Blocking: called on the web view's background thread.
     */
    fun inlinePart(
        messageId: Long,
        contentId: String,
    ): InlinePart?
}
