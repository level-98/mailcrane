package app.mailcrane.core

/**
 * JMAP push subscriptions (M4-27, RFC 8620 §7.2): the core registers the UnifiedPush endpoint
 * with every JMAP account, answers the servers' verification pushes and removes them again. An
 * interface so the push worker can be tested with a fake. The calls go over the network.
 */
interface JmapPushCore {
    /**
     * Registers [endpoint] (with the RFC 8291 keys, both or neither) with every JMAP account,
     * creating, refreshing or replacing subscriptions as needed. Returns how many JMAP accounts
     * have one afterwards; 0 when there are none or it failed.
     */
    suspend fun registerPush(
        endpoint: String,
        p256dh: String?,
        auth: String?,
    ): Int

    /** Answers a server's verification push. Returns whether [body] was one and was answered. */
    suspend fun answerPush(body: ByteArray): Boolean

    /** Destroys every JMAP push subscription: push wake-ups were turned off. */
    suspend fun unregisterPush()
}

/** Whether a push body is a JMAP verification to answer, or a wake-up. No I/O; any thread. */
fun isPushVerification(body: ByteArray): Boolean = classifyPush(body) == PushBody.VERIFICATION
