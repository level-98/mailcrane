package app.mailcrane.core

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.coroutineScope
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

    /**
     * Multi-select (M4-14): applies [action] to everything in [selection], across folders and accounts, all
     * or nothing, as one queued action per folder. With [undoable], the server copy waits [UNDO_WINDOW_MS]
     * so [undoBulk] can still take it back.
     * @throws ActionFailure
     */
    suspend fun bulkAction(
        selection: BulkSelection,
        action: MessageAction,
        undoable: Boolean,
    ): BulkApplied = BulkApplied(emptyList(), emptyList(), false)

    /** Undo for [bulkAction]: puts back the messages whose action hasn't reached the server. @throws ActionFailure */
    suspend fun undoBulk(messageIds: List<Long>): BulkUndo = BulkUndo(0u, 0u)

    /** How many messages in [selection] a Delete would delete for good (asked before deleting). @throws ActionFailure */
    suspend fun countDeletedForGood(selection: BulkSelection): Int = 0

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

    /** Searches the stored mail of every account (M4-21); each row names its account. */
    suspend fun searchAllAccounts(
        query: String,
        limit: Int,
    ): List<MessageSummary> = emptyList()

    /** The filters [query] uses, for the search chips (M4-21). Pure and quick: no I/O. */
    fun searchFilters(query: String): SearchFilters = NO_FILTERS

    /** [query] with [flag] turned on or off. */
    fun searchWithFlag(
        query: String,
        flag: SearchFlag,
        on: Boolean,
    ): String = query

    /** [query] without its `from:` terms. */
    fun searchWithoutFrom(query: String): String = query

    /** [query] limited to days [afterDay] (inclusive) to [beforeDay] (exclusive), days since 1970-01-01. */
    fun searchWithDates(
        query: String,
        afterDay: Long?,
        beforeDay: Long?,
    ): String = query

    /** [query] limited to accounts matching [account], or every account with null. */
    fun searchWithAccount(
        query: String,
        account: String?,
    ): String = query

    /**
     * Wakes every snooze that is due and starts moving those messages back; returns them, once each,
     * for a notification. @throws SnoozeFailure
     */
    suspend fun wakeDueSnoozes(): List<WokenMessage> = emptyList()

    /** The Unified Inbox: every account's inbox, newest first (M4-01). */
    suspend fun listUnifiedInbox(
        offset: Int,
        limit: Int,
    ): List<MessageSummary> = emptyList()

    /** Syncs every account's inbox; one account failing doesn't stop the others. @throws SyncFailure if all fail */
    suspend fun syncUnifiedInbox() {}

    /** Unread messages across every account's inbox. */
    suspend fun unifiedUnreadCount(): Int = 0

    /**
     * Runs a rule on the messages now in [folderId] of [accountId] and sends the changes to the server;
     * returns how many messages it changed (M4-09). @throws RulesFailure
     */
    suspend fun runRuleOnExisting(
        ruleId: Long,
        folderId: Long,
        accountId: Long,
    ): Int = 0

    /** Conversations with messages in the folder, newest first (M4-02). */
    suspend fun listThreads(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<ThreadSummary> = emptyList()

    /** Every message of a conversation across folders, oldest first. */
    suspend fun listThreadMessages(threadId: Long): List<MessageSummary> = emptyList()

    /** The next time a snoozed message wakes (Unix seconds), or null. @throws SnoozeFailure */
    suspend fun nextSnoozeWake(): Long? = null
}

/** The folder id the apps use for the Unified Inbox, every account's inbox together (M4-01). */
const val UNIFIED_FOLDER_ID = -1L

/** A query with no filters. */
val NO_FILTERS = SearchFilters(emptyList(), false, false, false, null, null, null)

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

/**
 * Calls the core through the generated UniFFI bindings. [dbPath] is the app's database file. Also
 * starts and stops live updates, so their events reach the same [events] flow.
 */
class UniffiMailboxCore(
    private val dbPath: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : MailboxCore,
    MessageCore,
    LiveUpdatesEngine,
    NotificationsCore {
    private val hub = EventHub()

    // Created on first use, so the native library loads only when mail is needed. The system
    // resolver finds the account's ManageSieve server through SRV (M4-23).
    private val engine by lazy { SyncEngine(dbPath, hub).also { it.setPlatformDns(AndroidDns()) } }

    override val events: Flow<CoreEvent> = hub.flow()

    /** Reports the device conditions for background downloads (M4-25). Cheap; any thread. */
    fun setDeviceConditions(conditions: DeviceConditions) {
        engine.setDeviceConditions(conditions)
    }

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

    override suspend fun takeNewMail(): List<NewMailNotification> = withContext(io) { engine.takeNewMail() }

    override suspend fun backgroundSync(deadlineSeconds: Int): List<NewMailNotification> =
        withContext(io) { engine.backgroundSync(deadlineSeconds.coerceAtLeast(1).toUInt()) }

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

    override suspend fun bulkAction(
        selection: BulkSelection,
        action: MessageAction,
        undoable: Boolean,
    ): BulkApplied {
        val applied = withContext(io) { engine.applyBulkAction(selection, action) }
        applied.accountIds.forEach { replaySoon(it, if (undoable) UNDO_WINDOW_MS + 500 else 0) }
        return applied
    }

    override suspend fun undoBulk(messageIds: List<Long>): BulkUndo =
        withContext(io) { engine.undoBulkAction(messageIds) }

    override suspend fun countDeletedForGood(selection: BulkSelection): Int =
        withContext(io) { engine.countDeletedForGood(selection) }.coerceAtMost(Int.MAX_VALUE.toUInt()).toInt()

    override suspend fun snooze(
        messageId: Long,
        wakeAt: Long,
    ) {
        withContext(io) { engine.snoozeMessages(listOf(messageId), wakeAt) }.forEach { replaySoon(it) }
    }

    override suspend fun unsnooze(messageId: Long) {
        withContext(io) { engine.unsnoozeMessages(listOf(messageId)) }.forEach { replaySoon(it) }
    }

    override suspend fun snoozeInfo(messageId: Long): SnoozeInfo? = withContext(io) { engine.snoozeInfo(messageId) }

    override suspend fun wakeDueSnoozes(): List<WokenMessage> {
        val woken = withContext(io) { engine.wakeDueSnoozes() }
        woken.map { it.accountId }.distinct().forEach { replaySoon(it) }
        return woken
    }

    override suspend fun nextSnoozeWake(): Long? = withContext(io) { engine.nextSnoozeWake() }

    override suspend fun listUnifiedInbox(
        offset: Int,
        limit: Int,
    ): List<MessageSummary> =
        withContext(io) {
            listUnifiedInbox(dbPath, offset.coerceAtLeast(0).toUInt(), limit.coerceAtLeast(0).toUInt())
        }

    override suspend fun syncUnifiedInbox() {
        val inboxes =
            withContext(io) {
                listAccounts(dbPath).mapNotNull { account ->
                    listFolders(dbPath, account.id).firstOrNull { it.role == FolderRole.INBOX }?.id
                }
            }
        // Accounts sync side by side; each has its own lock in the core.
        val failures =
            coroutineScope {
                inboxes
                    .map { inbox ->
                        async(io) {
                            try {
                                engine.syncFolder(inbox)
                                null
                            } catch (e: SyncFailure) {
                                e
                            }
                        }
                    }.awaitAll()
                    .filterNotNull()
            }
        if (inboxes.isNotEmpty() && failures.size == inboxes.size) throw failures.first()
    }

    override suspend fun unifiedUnreadCount(): Int =
        withContext(io) {
            try {
                unifiedUnreadCount(dbPath).coerceAtMost(Int.MAX_VALUE.toULong()).toInt()
            } catch (_: SyncFailure) {
                0
            }
        }

    override suspend fun runRuleOnExisting(
        ruleId: Long,
        folderId: Long,
        accountId: Long,
    ): Int {
        val changed = withContext(io) { engine.runRuleOnExisting(ruleId, folderId).toInt() }
        if (changed > 0) replaySoon(accountId)
        return changed
    }

    override suspend fun listThreads(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<ThreadSummary> =
        withContext(io) {
            listThreads(dbPath, folderId, offset.coerceAtLeast(0).toUInt(), limit.coerceAtLeast(0).toUInt())
        }

    override suspend fun listThreadMessages(threadId: Long): List<MessageSummary> =
        withContext(io) { listThreadMessages(dbPath, threadId) }

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

    override suspend fun searchAllAccounts(
        query: String,
        limit: Int,
    ): List<MessageSummary> = withContext(io) { searchAllAccounts(dbPath, query, limit.coerceAtLeast(0).toUInt()) }

    override fun searchFilters(query: String): SearchFilters = app.mailcrane.core.searchFilters(query)

    override fun searchWithFlag(
        query: String,
        flag: SearchFlag,
        on: Boolean,
    ): String = app.mailcrane.core.searchWithFlag(query, flag, on)

    override fun searchWithoutFrom(query: String): String = app.mailcrane.core.searchWithoutFrom(query)

    override fun searchWithDates(
        query: String,
        afterDay: Long?,
        beforeDay: Long?,
    ): String = app.mailcrane.core.searchWithDates(query, afterDay, beforeDay)

    override fun searchWithAccount(
        query: String,
        account: String?,
    ): String = app.mailcrane.core.searchWithAccount(query, account)

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

    override suspend fun saveAttachment(
        messageId: Long,
        partId: String,
        destDir: String,
    ): String = withContext(io) { saveAttachment(dbPath, messageId, partId, destDir) }

    override fun inlinePart(
        messageId: Long,
        contentId: String,
    ): InlinePart? = getInlinePart(dbPath, messageId, contentId)

    override suspend fun listFolders(accountId: Long): List<FolderSummary> =
        withContext(io) { listFolders(dbPath, accountId) }

    // Live updates: blocking calls, made off the main thread by LiveUpdatesObserver. Failures
    // also arrive as SyncError events.
    override fun accountIds(): List<Long> =
        try {
            engine.accountIds()
        } catch (e: SyncFailure) {
            emptyList()
        }

    override fun start(accountId: Long): Boolean =
        try {
            engine.startLiveUpdates(accountId)
            true
        } catch (e: SyncFailure) {
            false
        }

    override fun stop(accountId: Long) = engine.stopLiveUpdates(accountId)
}
