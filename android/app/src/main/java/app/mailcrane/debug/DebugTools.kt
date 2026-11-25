package app.mailcrane.debug

/**
 * Developer tools that exist only in debug builds (#289). The debug source set binds the real
 * tools; release builds bind [NoDebugTools], so release code has nothing to call.
 */
interface DebugTools {
    /** Whether this build has debug tools; `false` in release builds. */
    val available: Boolean

    /**
     * Replaces the inbox of the local-only Fixtures account (`fixtures@example.org`) with the test
     * messages from `tests/fixtures`, creating the account the first time. That account never
     * connects to a server. Returns how many messages its inbox now holds; throws on failure.
     */
    suspend fun loadFixtures(): Int
}

/** Release builds: no debug tools. */
object NoDebugTools : DebugTools {
    override val available: Boolean = false

    override suspend fun loadFixtures(): Int = 0
}
