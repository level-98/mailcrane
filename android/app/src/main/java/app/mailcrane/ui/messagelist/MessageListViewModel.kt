package app.mailcrane.ui.messagelist

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.BulkSelection
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.ThreadSummary
import app.mailcrane.core.UNDO_WINDOW_MS
import app.mailcrane.core.UNIFIED_FOLDER_ID
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.DEFAULT_SWIPE_LEFT
import app.mailcrane.settings.DEFAULT_SWIPE_RIGHT
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.folders.takesMoves
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

data class MessageListState(
    /** The loaded part of the folder, newest first, as the core returned it. */
    val messages: List<MessageSummary> = emptyList(),
    /** The first read from the database hasn't finished. */
    val isLoading: Boolean = true,
    /** Pull-to-refresh (a folder sync) is running. */
    val isRefreshing: Boolean = false,
    /** Older messages are being loaded. */
    val isLoadingOlder: Boolean = false,
    /** The server has no older messages. */
    val endReached: Boolean = false,
    /** Reading the database failed and nothing could be shown. */
    val loadError: SyncErrorKind? = null,
    /** Show conversations instead of single messages (M4-02). */
    val grouped: Boolean = false,
    /** The conversations covering the loaded messages, newest first, when [grouped]. */
    val threads: List<ThreadSummary> = emptyList(),
    /** Selected message rows (M4-14). */
    val selectedMessages: Set<Long> = emptySet(),
    /** Selected conversation rows, by thread id, when [grouped]. */
    val selectedThreads: Set<Long> = emptySet(),
    /** How many selected messages a Delete would delete for good, while asking first; 0 when not asking. */
    val confirmDeleteForGood: Int = 0,
) {
    /** Multi-select is on: taps toggle rows and the top bar shows the bulk actions. */
    val selecting: Boolean get() = selectedMessages.isNotEmpty() || selectedThreads.isNotEmpty()

    /** How many rows are selected. */
    val selectedCount: Int get() = selectedMessages.size + selectedThreads.size

    /** Every selected row is read, so the top bar offers Mark unread. */
    val allSelectedRead: Boolean
        get() =
            messages.filter { it.id in selectedMessages }.all { it.isSeen } &&
                threads.filter { it.threadId in selectedThreads }.all { it.unreadCount == 0u }

    /** Every selected row is flagged, so the top bar offers Unflag. */
    val allSelectedFlagged: Boolean
        get() =
            messages.filter { it.id in selectedMessages }.all { it.isFlagged } &&
                threads.filter { it.threadId in selectedThreads }.all { it.isFlagged }

    /** The accounts of the selected rows (0 for rows outside the Unified Inbox, which name none). */
    val selectedAccounts: Set<Long>
        get() = messages.filter { it.id in selectedMessages }.mapTo(HashSet()) { it.accountId }
}

/** What the selection top bar can do to every selected row (M4-14). */
enum class BulkAction { READ, UNREAD, FLAG, UNFLAG, ARCHIVE, DELETE }

/**
 * The message list of one folder. All mail logic stays in the core: this only asks it for pages,
 * syncs and older messages, and re-reads when the core says something changed.
 */
@HiltViewModel(assistedFactory = MessageListViewModel.Factory::class)
class MessageListViewModel
    @AssistedInject
    constructor(
        private val core: MailboxCore,
        settings: AppSettings,
        @Assisted private val folderId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(folderId: Long): MessageListViewModel
        }

        /** The Unified Inbox (M4-01) rather than one folder. */
        private val unified = folderId == UNIFIED_FOLDER_ID

        private val _state = MutableStateFlow(MessageListState())
        val state: StateFlow<MessageListState> = _state.asStateFlow()

        private val _errors = Channel<SyncErrorKind>(Channel.BUFFERED)

        /**
         * One-off sync failures, shown in a snackbar. [SyncErrorKind.BUSY] is left out: another sync
         * was saving mail at the same time and fetches it anyway (issue #282).
         */
        val errors: Flow<SyncErrorKind> = _errors.receiveAsFlow().filter { it != SyncErrorKind.BUSY }

        private val _actionsUndone = Channel<Int>(Channel.CONFLATED)

        /** The swipe settings (right, left). */
        val swipes: StateFlow<Pair<SwipeAction, SwipeAction>> =
            combine(settings.swipeRight, settings.swipeLeft) { r, l -> r to l }
                .stateIn(viewModelScope, SharingStarted.Eagerly, DEFAULT_SWIPE_RIGHT to DEFAULT_SWIPE_LEFT)

        private val _undoOffers = Channel<UndoOffer>(Channel.BUFFERED)

        /** "Archived · Undo" and "Deleted · Undo", shown for [UNDO_WINDOW_MS]. */
        val undoOffers: Flow<UndoOffer> = _undoOffers.receiveAsFlow()

        private val _notices = Channel<Int>(Channel.BUFFERED)

        init {
            // Follows the "Group by conversation" setting.
            viewModelScope.launch {
                settings.groupByConversation.distinctUntilChanged().collect { grouped ->
                    _state.update { it.copy(grouped = grouped) }
                    refreshThreads()
                    pruneSelection()
                }
            }
        }

        private suspend fun read(
            offset: Int,
            limit: Int,
        ): List<MessageSummary> =
            if (unified) core.listUnifiedInbox(offset, limit) else core.listMessages(folderId, offset, limit)

        /** Re-reads the conversations for the loaded range, when grouping is on. */
        private suspend fun refreshThreads() {
            // Conversations are per folder; the Unified Inbox lists messages.
            if (!_state.value.grouped || unified) {
                _state.update { it.copy(threads = emptyList()) }
                return
            }
            try {
                val threads = core.listThreads(folderId, 0, window.coerceAtMost(READ_CHUNK * 2))
                _state.update { it.copy(threads = threads) }
            } catch (e: SyncFailure.Failed) {
                _errors.send(e.kind)
            }
        }

        /** One-off messages from swipes, as string resources. */
        val notices: Flow<Int> = _notices.receiveAsFlow()

        private val _bulkUndoOffers = Channel<BulkUndoOffer>(Channel.BUFFERED)

        /** "3 archived · Undo" after a bulk Archive, Delete or Move, shown for [UNDO_WINDOW_MS]. */
        val bulkUndoOffers: Flow<BulkUndoOffer> = _bulkUndoOffers.receiveAsFlow()

        /** Long-press, or a tap while selecting: selects or deselects a message row. */
        fun toggleMessage(id: Long) =
            _state.update {
                it.copy(
                    selectedMessages =
                        if (id in
                            it.selectedMessages
                        ) {
                            it.selectedMessages - id
                        } else {
                            it.selectedMessages + id
                        },
                )
            }

        /** Selects or deselects a conversation row. */
        fun toggleThread(threadId: Long) =
            _state.update {
                it.copy(
                    selectedThreads =
                        if (threadId in
                            it.selectedThreads
                        ) {
                            it.selectedThreads - threadId
                        } else {
                            it.selectedThreads + threadId
                        },
                )
            }

        /** Back or the top bar's Clear: ends selection. */
        fun clearSelection() =
            _state.update {
                it.copy(
                    selectedMessages = emptySet(),
                    selectedThreads = emptySet(),
                    confirmDeleteForGood = 0,
                )
            }

        /** Runs [action] on every selected row through the core, then ends selection. */
        fun bulk(action: BulkAction) {
            // Delete may be permanent: it asks first when it would be.
            if (action == BulkAction.DELETE) return requestDelete()
            val coreAction =
                when (action) {
                    BulkAction.READ -> MessageAction.MarkRead
                    BulkAction.UNREAD -> MessageAction.MarkUnread
                    BulkAction.FLAG -> MessageAction.Flag
                    BulkAction.UNFLAG -> MessageAction.Unflag
                    BulkAction.ARCHIVE -> MessageAction.Archive
                    BulkAction.DELETE -> MessageAction.Delete
                }
            val kind =
                when (action) {
                    BulkAction.ARCHIVE -> BulkUndoKind.ARCHIVED
                    BulkAction.DELETE -> BulkUndoKind.DELETED
                    else -> null
                }
            runBulk(coreAction, kind)
        }

        private val _moveTargets = MutableStateFlow<List<FolderSummary>?>(null)

        /** The folders to offer for the selection's Move while its dialog is open, else null. */
        val moveTargets: StateFlow<List<FolderSummary>?> = _moveTargets.asStateFlow()

        /**
         * The selection's Move: offers [folders] (this folder's account, minus this folder), or in the Unified
         * Inbox the selected messages' account's folders. Messages from several accounts can't share a
         * destination, so that says so instead.
         */
        fun startMove(folders: List<FolderSummary>) {
            if (!unified) {
                _moveTargets.value = folders.filter { it.id != folderId && it.takesMoves }
                return
            }
            val accounts = _state.value.selectedAccounts
            val account = accounts.singleOrNull()
            viewModelScope.launch {
                if (account == null) {
                    _notices.send(R.string.bulk_move_one_account)
                    return@launch
                }
                try {
                    _moveTargets.value =
                        core.listFolders(account).filter { it.role != FolderRole.INBOX && it.takesMoves }
                } catch (_: SyncFailure) {
                    _notices.send(R.string.action_not_applied)
                }
            }
        }

        /** The Move dialog was closed without a pick. */
        fun cancelMove() {
            _moveTargets.value = null
        }

        /** The selection's Move, once a folder is picked. */
        fun bulkMove(folderId: Long) {
            _moveTargets.value = null
            runBulk(MessageAction.Move(folderId), BulkUndoKind.MOVED)
        }

        /**
         * The selection's Delete: asks the core how many messages it would delete for good (already in Trash,
         * or in an account without one). If any, the screen asks first; otherwise they go to Trash at once.
         */
        private fun requestDelete() {
            val selection = currentSelection() ?: return
            viewModelScope.launch {
                val forGood =
                    try {
                        core.countDeletedForGood(selection)
                    } catch (_: ActionFailure) {
                        _notices.send(R.string.action_not_applied)
                        return@launch
                    }
                if (forGood > 0) {
                    _state.update { it.copy(confirmDeleteForGood = forGood) }
                } else {
                    runBulk(MessageAction.Delete, BulkUndoKind.DELETED)
                }
            }
        }

        /** "Delete for good" in the dialog: deletes with no Undo, since a permanent delete can't promise one. */
        fun confirmDeleteForGood() {
            _state.update { it.copy(confirmDeleteForGood = 0) }
            runBulk(MessageAction.Delete, undo = null)
        }

        /** The dialog was dismissed: nothing is deleted and the selection stays. */
        fun cancelDeleteForGood() = _state.update { it.copy(confirmDeleteForGood = 0) }

        private fun currentSelection(): BulkSelection? {
            val current = _state.value
            if (!current.selecting) return null
            return BulkSelection(
                messageIds = current.selectedMessages.toList(),
                folderId = folderId,
                threadIds = current.selectedThreads.toList(),
            )
        }

        private fun runBulk(
            action: MessageAction,
            undo: BulkUndoKind?,
        ) {
            val selection = currentSelection() ?: return
            clearSelection()
            viewModelScope.launch {
                try {
                    val applied = core.bulkAction(selection, action, undoable = undo != null)
                    when {
                        applied.messageIds.isEmpty() -> Unit
                        // Said, never silent, but with no Undo: it can't be promised for a permanent delete.
                        applied.deletedForGood ->
                            _bulkUndoOffers.send(BulkUndoOffer(applied.messageIds, BulkUndoKind.DELETED_FOR_GOOD))
                        undo != null -> _bulkUndoOffers.send(BulkUndoOffer(applied.messageIds, undo))
                    }
                } catch (e: ActionFailure.NoArchiveFolder) {
                    _notices.send(R.string.action_no_archive_folder)
                } catch (e: ActionFailure.MixedAccounts) {
                    _notices.send(R.string.bulk_move_one_account)
                } catch (e: ActionFailure) {
                    _notices.send(R.string.action_not_applied)
                }
            }
        }

        /** The bulk snackbar's Undo. */
        fun undoBulk(messageIds: List<Long>) {
            viewModelScope.launch {
                val result =
                    try {
                        core.undoBulk(messageIds)
                    } catch (_: ActionFailure) {
                        null
                    }
                when {
                    result == null -> _notices.send(R.string.action_not_applied)
                    result.undone == 0u -> _notices.send(R.string.swipe_undo_too_late)
                    result.tooLate > 0u -> _notices.send(R.string.bulk_undo_partly_late)
                }
            }
        }

        /** Runs [action] on [message] through the core (never by changing the list alone). Not for MOVE. */
        fun swipe(
            message: MessageSummary,
            action: SwipeAction,
        ) {
            val (coreAction, undoable) =
                when (action) {
                    SwipeAction.NONE, SwipeAction.MOVE -> return
                    SwipeAction.ARCHIVE -> MessageAction.Archive to true
                    SwipeAction.DELETE -> MessageAction.Delete to true
                    SwipeAction.READ_UNREAD ->
                        (if (message.isSeen) MessageAction.MarkUnread else MessageAction.MarkRead) to false
                    SwipeAction.FLAG -> (if (message.isFlagged) MessageAction.Unflag else MessageAction.Flag) to false
                }
            run(message.id, coreAction, undoable, action)
        }

        /** The Move swipe, once a folder is picked. */
        fun move(
            messageId: Long,
            folderId: Long,
        ) = run(messageId, MessageAction.Move(folderId), undoable = false, swipe = SwipeAction.MOVE)

        /** The snackbar's Undo. */
        fun undo(messageId: Long) {
            viewModelScope.launch {
                val undone =
                    try {
                        core.undoAction(messageId)
                    } catch (_: ActionFailure) {
                        false
                    }
                if (!undone) _notices.send(R.string.swipe_undo_too_late)
            }
        }

        private fun run(
            messageId: Long,
            action: MessageAction,
            undoable: Boolean,
            swipe: SwipeAction,
        ) {
            viewModelScope.launch {
                try {
                    core.swipeAction(messageId, action, undoable)
                    if (undoable) _undoOffers.send(UndoOffer(messageId, swipe))
                } catch (e: ActionFailure.NoArchiveFolder) {
                    _notices.send(R.string.action_no_archive_folder)
                } catch (e: ActionFailure) {
                    _notices.send(R.string.action_not_applied)
                }
            }
        }

        /** The server refused a message action and it was undone; shown in a snackbar. */
        /** What to tell the user when a queued change couldn't be applied: a string resource. */
        val actionsUndone: Flow<Int> = _actionsUndone.receiveAsFlow()

        /** How many of the newest messages the list holds; grows as you scroll. */
        private var window = PAGE_SIZE
        private val reading = Mutex()

        init {
            viewModelScope.launch {
                core.events
                    .filter { it.isAbout(folderId) }
                    .collect { reload() }
            }
            viewModelScope.launch {
                core.events
                    .filter { it is CoreEvent.ActionFailed }
                    .collect { _actionsUndone.send(undoneMessage(it as CoreEvent.ActionFailed)) }
            }
            viewModelScope.launch {
                reload()
                refresh()
            }
        }

        /** Syncs the folder with the server; progress shows until it finishes or fails. */
        fun refresh() {
            if (_state.value.isRefreshing) return
            _state.update { it.copy(isRefreshing = true) }
            viewModelScope.launch {
                try {
                    if (unified) core.syncUnifiedInbox() else core.syncFolder(folderId)
                    reload()
                } catch (e: SyncFailure.Failed) {
                    _errors.send(e.kind)
                } finally {
                    _state.update { it.copy(isRefreshing = false) }
                }
            }
        }

        /**
         * The list is near its end. Shows more stored messages if there are any, otherwise asks the
         * server for one more page. Does nothing while a call is running or once the end is reached.
         */
        fun loadMore() {
            val current = _state.value
            if (current.isLoading || current.isLoadingOlder || current.endReached) return
            _state.update { it.copy(isLoadingOlder = true) }
            viewModelScope.launch {
                try {
                    val shown = _state.value.messages.size
                    val moreStored = shown >= window
                    window = shown + PAGE_SIZE
                    if (moreStored) appendStored()
                    if (_state.value.messages.size == shown) {
                        val stored = if (unified) 0 else core.loadOlder(folderId, PAGE_SIZE)
                        if (stored == 0) {
                            _state.update { it.copy(endReached = true) }
                        } else {
                            reload()
                        }
                    }
                } catch (e: SyncFailure.Failed) {
                    _errors.send(e.kind)
                } finally {
                    _state.update { it.copy(isLoadingOlder = false) }
                }
            }
        }

        /** Drops selected rows that left the list (moved, deleted, or changed elsewhere). */
        private fun pruneSelection() =
            _state.update { s ->
                if (!s.selecting) return@update s.copy(confirmDeleteForGood = 0)
                val ids = s.messages.mapTo(HashSet()) { it.id }
                val threads = s.threads.mapTo(HashSet()) { it.threadId }
                s.copy(
                    selectedMessages = s.selectedMessages.filterTo(HashSet()) { it in ids },
                    // Conversation rows only show while grouping is on.
                    selectedThreads =
                        if (s.grouped) {
                            s.selectedThreads.filterTo(
                                HashSet(),
                            ) { it in threads }
                        } else {
                            emptySet()
                        },
                )
            }

        /** Re-reads the loaded range from the database. */
        private suspend fun reload() {
            reading.withLock {
                try {
                    val messages = readWindow()
                    _state.update { it.copy(messages = messages, isLoading = false, loadError = null) }
                    refreshThreads()
                    pruneSelection()
                } catch (e: SyncFailure.Failed) {
                    if (_state.value.messages.isEmpty()) {
                        _state.update { it.copy(isLoading = false, loadError = e.kind) }
                    } else {
                        _errors.send(e.kind)
                    }
                }
            }
        }

        /**
         * Reads only the next page after the loaded ones, so scrolling a large folder doesn't re-read
         * everything above. A change while reading is caught by the core event that follows it.
         */
        private suspend fun appendStored() {
            reading.withLock {
                val loaded = _state.value.messages
                val next = read(loaded.size, window - loaded.size)
                // A reload may have run meanwhile; drop rows already in the list.
                val known = loaded.mapTo(HashSet(loaded.size)) { it.id }
                _state.update { it.copy(messages = loaded + next.filter { m -> m.id !in known }) }
                refreshThreads()
            }
        }

        /** The newest [window] messages, read in chunks the core accepts. */
        private suspend fun readWindow(): List<MessageSummary> {
            val out = ArrayList<MessageSummary>(window)
            while (out.size < window) {
                val want = minOf(READ_CHUNK, window - out.size)
                val chunk = read(out.size, want)
                out += chunk
                if (chunk.size < want) break
            }
            return out
        }

        companion object {
            /** Messages added per scroll step, and per `load_older` call. */
            const val PAGE_SIZE = 50

            /** Rows per database read; at most the core's limit of 500. */
            const val READ_CHUNK = 250

            /** Ask for more when the last visible row is this close to the end. */
            const val LOAD_MORE_THRESHOLD = 20
        }
    }

/** What a bulk action did, for its snackbar. */
enum class BulkUndoKind { ARCHIVED, DELETED, DELETED_FOR_GOOD, MOVED }

/** Undo is offered for [messageIds] after a bulk action (M4-14); never for [BulkUndoKind.DELETED_FOR_GOOD]. */
data class BulkUndoOffer(
    val messageIds: List<Long>,
    val kind: BulkUndoKind,
)

/** Undo is offered for [messageId] after [action] (archive or delete). */
data class UndoOffer(
    val messageId: Long,
    val action: SwipeAction,
)

private fun CoreEvent.isAbout(folderId: Long): Boolean =
    when (this) {
        // The Unified Inbox shows several folders: any change may concern it.
        is CoreEvent.MessagesChanged -> folderId == UNIFIED_FOLDER_ID || this.folderId == folderId
        is CoreEvent.FolderChanged -> folderId == UNIFIED_FOLDER_ID || this.folderId == folderId
        is CoreEvent.SyncError -> false
        is CoreEvent.ActionFailed -> false
    }

/**
 * The message for a change that couldn't be applied: the server refused it (and it was undone), or the server
 * reset the folder, so changes queued there no longer point at the right messages (#517).
 */
internal fun undoneMessage(failed: CoreEvent.ActionFailed): Int =
    if (failed.reason == SyncErrorKind.FOLDER_RESET) R.string.action_dropped_folder_reset else R.string.action_undone
