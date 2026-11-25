package app.mailcrane.ui.messagelist

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.ThreadSummary
import app.mailcrane.core.UNDO_WINDOW_MS
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.DEFAULT_SWIPE_LEFT
import app.mailcrane.settings.DEFAULT_SWIPE_RIGHT
import app.mailcrane.settings.SwipeAction
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
)

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

        private val _state = MutableStateFlow(MessageListState())
        val state: StateFlow<MessageListState> = _state.asStateFlow()

        private val _errors = Channel<SyncErrorKind>(Channel.BUFFERED)

        /** One-off sync failures, shown in a snackbar. */
        val errors: Flow<SyncErrorKind> = _errors.receiveAsFlow()

        private val _actionsUndone = Channel<Unit>(Channel.CONFLATED)

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
                }
            }
        }

        /** Re-reads the conversations for the loaded range, when grouping is on. */
        private suspend fun refreshThreads() {
            if (!_state.value.grouped) {
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
        val actionsUndone: Flow<Unit> = _actionsUndone.receiveAsFlow()

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
                    .collect { _actionsUndone.send(Unit) }
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
                    core.syncFolder(folderId)
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
                        val stored = core.loadOlder(folderId, PAGE_SIZE)
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

        /** Re-reads the loaded range from the database. */
        private suspend fun reload() {
            reading.withLock {
                try {
                    val messages = readWindow()
                    _state.update { it.copy(messages = messages, isLoading = false, loadError = null) }
                    refreshThreads()
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
                val next = core.listMessages(folderId, loaded.size, window - loaded.size)
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
                val chunk = core.listMessages(folderId, out.size, want)
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

/** Undo is offered for [messageId] after [action] (archive or delete). */
data class UndoOffer(
    val messageId: Long,
    val action: SwipeAction,
)

private fun CoreEvent.isAbout(folderId: Long): Boolean =
    when (this) {
        is CoreEvent.MessagesChanged -> this.folderId == folderId
        is CoreEvent.FolderChanged -> this.folderId == folderId
        is CoreEvent.SyncError -> false
        is CoreEvent.ActionFailed -> false
    }
