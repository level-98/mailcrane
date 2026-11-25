package app.mailcrane.core

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.concurrent.ConcurrentHashMap
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

    /** The account's stored folders in display order, with unread counts. */
    suspend fun listFolders(accountId: Long): List<FolderSummary>

    /**
     * A swipe action: applied at once and queued like [MessageCore.applyAction]. With [undoable], the
     * server copy waits [UNDO_WINDOW_MS] first so [undoAction] can still take it back.
     * @throws ActionFailure
     */
    suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    )

    /** Undoes the newest action on the message if it hasn't reached the server; false if too late. */
    suspend fun undoAction(messageId: Long): Boolean

    /** Searches the stored mail (works offline), newest first. */
    suspend fun searchMessages(
        accountId: Long,
        folderId: Long?,
        query: String,
        limit: Int,
    ): List<MessageSummary>

    /** Asks the server for matches in the folder that were never downloaded; returns how many were stored. */
    suspend fun searchServer(
        folderId: Long,
        query: String,
    ): Int
}

/** How long Undo is offered after archiving or deleting with a swipe. */
const val UNDO_WINDOW_MS = 5_000L

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
) : MailboxCore,
    MessageCore {
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

    override suspend fun getMessage(
        messageId: Long,
        allowRemote: Boolean,
    ): MessageDetail = withContext(io) { getMessage(dbPath, messageId, allowRemote) }

    override suspend fun fetchBody(messageId: Long) {
        withContext(io) { engine.fetchBody(messageId) }
    }

    override suspend fun markRead(messageId: Long) {
        try {
            applyAction(messageId, MessageAction.MarkRead)
        } catch (_: ActionFailure.NotFound) {
            throw SyncFailure.Failed(SyncErrorKind.NOT_FOUND)
        } catch (_: ActionFailure) {
            throw SyncFailure.Failed(SyncErrorKind.STORAGE)
        }
    }

    override suspend fun applyAction(
        messageId: Long,
        action: MessageAction,
    ) {
        val account = withContext(io) { engine.applyAction(listOf(messageId), action) }
        if (account != null) replaySoon(account)
    }

    override suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    ) {
        val account = withContext(io) { engine.applyAction(listOf(messageId), action) }
        if (account != null) replaySoon(account, if (undoable) UNDO_WINDOW_MS + 500 else 0)
    }

    override suspend fun undoAction(messageId: Long): Boolean = withContext(io) { engine.undoAction(messageId) }

    override suspend fun searchMessages(
        accountId: Long,
        folderId: Long?,
        query: String,
        limit: Int,
    ): List<MessageSummary> =
        withContext(io) { searchMessages(dbPath, accountId, folderId, query, limit.coerceAtLeast(0).toUInt()) }

    override suspend fun searchServer(
        folderId: Long,
        query: String,
    ): Int = withContext(io) { engine.searchServer(folderId, query).toInt() }

    // Replays outlive the screen that started them (an archived message's screen closes at once).
    private val background = CoroutineScope(SupervisorJob() + io)
    private val replays = ConcurrentHashMap<Long, Job>()

    /**
     * Sends the account's queued actions to the server now, then again with back-off while some are
     * left (offline, or the server busy). One loop per account; a newer action restarts it.
     */
    private fun replaySoon(
        accountId: Long,
        firstDelayMs: Long = 0,
    ) {
        replays[accountId]?.cancel()
        replays[accountId] =
            background.launch {
                delay(firstDelayMs)
                var wait = FIRST_RETRY_MS
                while (true) {
                    val left =
                        try {
                            engine.replayActions(accountId).toInt()
                        } catch (_: SyncFailure) {
                            -1
                        }
                    if (left == 0) break
                    delay(wait)
                    wait = (wait * 2).coerceAtMost(MAX_RETRY_MS)
                }
            }
    }

    private companion object {
        const val FIRST_RETRY_MS = 5_000L
        const val MAX_RETRY_MS = 600_000L
    }

    override suspend fun unsubscribeOptions(messageId: Long): UnsubscribeInfo? =
        withContext(io) {
            try {
                unsubscribeOptions(dbPath, messageId)
            } catch (_: UnsubscribeFailure) {
                null
            }
        }

    override suspend fun oneClickUnsubscribe(messageId: Long) {
        withContext(io) { oneClickUnsubscribe(dbPath, messageId) }
    }

    override fun inlinePart(
        messageId: Long,
        contentId: String,
    ): InlinePart? = getInlinePart(dbPath, messageId, contentId)

    override suspend fun listFolders(accountId: Long): List<FolderSummary> =
        withContext(io) { listFolders(dbPath, accountId) }
}
