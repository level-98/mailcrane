package app.mailcrane.core

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withContext
import java.util.concurrent.CopyOnWriteArraySet

/**
 * Reading and syncing mail through the core. An interface so view models can be tested with a fake.
 * Every call runs off the main thread. Sync calls throw [SyncFailure] when they fail.
 */
interface MailboxCore {
    /** Core events (messages or folders changed, sync errors) for as long as the flow is collected. */
    val events: Flow<CoreEvent>

    /** Up to [limit] stored messages of the folder, newest first, after skipping [offset]. */
    suspend fun listMessages(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<MessageSummary>

    /** Fetches the newest messages of the folder from the server. */
    suspend fun syncFolder(folderId: Long)

    /** Fetches up to [count] messages older than the oldest stored one; returns how many were stored. */
    suspend fun loadOlder(
        folderId: Long,
        count: Int,
    ): Int

    /** Lists and saves the account's folders; returns the inbox's folder id, if it has one. */
    suspend fun refreshFolders(accountId: Long): Long?
}

/** Forwards events from the core to every collector of [flow]. Called by the core on any thread. */
private class EventHub : CoreEventListener {
    private val sinks = CopyOnWriteArraySet<(CoreEvent) -> Unit>()

    override fun onEvent(event: CoreEvent) {
        sinks.forEach { it(event) }
    }

    fun flow(): Flow<CoreEvent> =
        callbackFlow {
            val sink: (CoreEvent) -> Unit = { trySend(it) }
            sinks += sink
            awaitClose { sinks -= sink }
            // Never block the core's thread; events are small and rare.
        }.buffer(Channel.UNLIMITED)
}

/** Calls the core through the generated UniFFI bindings. [dbPath] is the app's database file. */
class UniffiMailboxCore(
    private val dbPath: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : MailboxCore {
    private val hub = EventHub()

    // Created on first use, so the native library loads only when mail is needed.
    private val engine by lazy { SyncEngine(dbPath, hub) }

    override val events: Flow<CoreEvent> = hub.flow()

    override suspend fun listMessages(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<MessageSummary> =
        withContext(io) {
            listMessages(dbPath, folderId, offset.coerceAtLeast(0).toUInt(), limit.coerceAtLeast(0).toUInt())
        }

    override suspend fun syncFolder(folderId: Long) {
        withContext(io) { engine.syncFolder(folderId) }
    }

    override suspend fun loadOlder(
        folderId: Long,
        count: Int,
    ): Int =
        withContext(io) {
            engine.loadOlder(folderId, count.coerceAtLeast(0).toUInt()).toInt().coerceAtLeast(0)
        }

    override suspend fun refreshFolders(accountId: Long): Long? = withContext(io) { engine.refreshFolders(accountId) }
}
