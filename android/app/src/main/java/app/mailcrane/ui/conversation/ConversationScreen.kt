package app.mailcrane.ui.conversation

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncFailure
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.DEFAULT_SWIPE_LEFT
import app.mailcrane.settings.DEFAULT_SWIPE_RIGHT
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.folders.takesMoves
import app.mailcrane.ui.message.MoveDialog
import app.mailcrane.ui.messagelist.MessageRow
import app.mailcrane.ui.messagelist.RowDateStyle
import app.mailcrane.ui.messagelist.SwipeHandlers
import app.mailcrane.ui.messagelist.SwipeableRow
import app.mailcrane.ui.messagelist.UndoOffer
import app.mailcrane.ui.messagelist.currentMinute
import app.mailcrane.ui.messagelist.runSwipeAction
import app.mailcrane.ui.messagelist.showSwipeUndo
import app.mailcrane.ui.messagelist.swipeA11yLabel
import app.mailcrane.ui.messagelist.swipeCoreAction
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
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The messages of one conversation, oldest first, across folders (M4-02). */
@HiltViewModel(assistedFactory = ConversationViewModel.Factory::class)
class ConversationViewModel
    @AssistedInject
    constructor(
        private val core: MailboxCore,
        settings: AppSettings,
        @Assisted private val threadId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(threadId: Long): ConversationViewModel
        }

        private val _messages = MutableStateFlow<List<MessageSummary>?>(null)

        /** Null while loading. */
        val messages: StateFlow<List<MessageSummary>?> = _messages.asStateFlow()

        init {
            viewModelScope.launch { load() }
            viewModelScope.launch {
                core.events.collect { if (it is CoreEvent.MessagesChanged) load() }
            }
        }

        /** The swipe settings (right, left): a conversation's messages swipe as list rows do (#552). */
        val swipes: StateFlow<Pair<SwipeAction, SwipeAction>> =
            combine(settings.swipeRight, settings.swipeLeft) { r, l -> r to l }
                .stateIn(viewModelScope, SharingStarted.Eagerly, DEFAULT_SWIPE_RIGHT to DEFAULT_SWIPE_LEFT)

        private val _undo = MutableStateFlow(SwipeUndo())

        /**
         * Swipes still waiting for the core, and the Undo to offer for the last one that removed a message.
         * State, not a stream: the screen waits for the swipes and hands the offer to the list if it closes
         * (#571), and nothing is left over for the next time the conversation opens.
         */
        val undo: StateFlow<SwipeUndo> = _undo.asStateFlow()

        /** The screen has shown [offer], or handed it to the list. */
        fun offerDone(offer: UndoOffer) = _undo.update { if (it.offer == offer) it.copy(offer = null) else it }

        private val _notices = Channel<Int>(Channel.BUFFERED)

        /** One-off messages from swipes, as string resources. */
        val notices: Flow<Int> = _notices.receiveAsFlow()

        private val _moveTargets = MutableStateFlow<Pair<Long, List<FolderSummary>>?>(null)

        /** While a Move swipe's dialog is open: the message, and the folders of its account to offer. */
        val moveTargets: StateFlow<Pair<Long, List<FolderSummary>>?> = _moveTargets.asStateFlow()

        /**
         * Runs [action] on [message] through the core, as a swipe in the list does. Move asks for a folder.
         * Archive leaves a message that is already archived alone and says so (#571).
         */
        fun swipe(
            message: MessageSummary,
            action: SwipeAction,
        ) {
            when (action) {
                SwipeAction.MOVE -> startMove(message)
                SwipeAction.ARCHIVE -> archive(message)
                else -> {
                    val (coreAction, undoable) = swipeCoreAction(message, action) ?: return
                    run(message.id, coreAction, undoable, action)
                }
            }
        }

        /** Each account's folders, read once: a conversation's messages all belong to one account. */
        private val folders = mutableMapOf<Long, List<FolderSummary>>()

        private suspend fun foldersOf(accountId: Long): List<FolderSummary> =
            folders[accountId] ?: core.listFolders(accountId).also { folders[accountId] = it }

        private fun startMove(message: MessageSummary) {
            viewModelScope.launch {
                try {
                    // The conversation spans folders: offer the message's account's folders, except its own.
                    _moveTargets.value =
                        message.id to
                        foldersOf(message.accountId).filter { it.takesMoves && it.id != message.folderId }
                } catch (_: SyncFailure) {
                    _notices.send(R.string.action_not_applied)
                }
            }
        }

        private fun archive(message: MessageSummary) {
            viewModelScope.launch {
                val role =
                    try {
                        foldersOf(message.accountId).find { it.id == message.folderId }?.role
                    } catch (_: SyncFailure) {
                        null
                    }
                // Already in Archive (or Gmail's All Mail): the core would change nothing, so say that rather
                // than "Archived".
                if (role == FolderRole.ARCHIVE || role == FolderRole.ALL_MAIL) {
                    _notices.send(R.string.conversation_already_archived)
                } else {
                    run(message.id, MessageAction.Archive, undoable = true, swipe = SwipeAction.ARCHIVE)
                }
            }
        }

        /** The Move swipe, once a folder is picked. */
        fun move(folderId: Long) {
            val messageId = _moveTargets.value?.first ?: return
            _moveTargets.value = null
            run(messageId, MessageAction.Move(folderId), undoable = false, swipe = SwipeAction.MOVE)
        }

        /** The Move dialog was closed without a pick. */
        fun cancelMove() {
            _moveTargets.value = null
        }

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
            _undo.update { it.copy(inFlight = it.inFlight + 1) }
            viewModelScope.launch {
                val refused = runSwipeAction(core, messageId, action, undoable)
                _undo.update {
                    it.copy(
                        inFlight = it.inFlight - 1,
                        offer = if (refused == null && undoable) UndoOffer(messageId, swipe) else it.offer,
                    )
                }
                if (refused != null) _notices.send(refused)
            }
        }

        private suspend fun load() {
            _messages.value =
                try {
                    core.listThreadMessages(threadId)
                } catch (_: SyncFailure) {
                    emptyList()
                }
        }
    }

/** Swipes in a conversation still waiting for the core, and the Undo to offer. */
data class SwipeUndo(
    val inFlight: Int = 0,
    val offer: UndoOffer? = null,
)

@Composable
fun ConversationScreen(
    threadId: Long,
    title: String,
    onBack: () -> Unit,
    onOpenMessage: (Long) -> Unit,
    /** The message ids shown, in order, for "After deleting or moving" (M4-15). */
    onShownMessages: (List<Long>) -> Unit = {},
    /** Takes an Undo still on offer when the conversation closes, so the list shows it (#571). */
    onUndoHandOff: (UndoOffer) -> Unit = {},
    viewModel: ConversationViewModel =
        hiltViewModel<ConversationViewModel, ConversationViewModel.Factory>(
            key = "thread-$threadId",
            creationCallback = { factory -> factory.create(threadId) },
        ),
) {
    val messages by viewModel.messages.collectAsStateWithLifecycle()
    val list = messages
    val undo by viewModel.undo.collectAsStateWithLifecycle()
    // A conversation that became empty (all moved away) closes, once its swipes are done; an Undo still on
    // offer goes to the list rather than being lost with the screen (#571).
    LaunchedEffect(list, undo.inFlight) {
        if (list != null && list.isEmpty() && undo.inFlight == 0) {
            undo.offer?.let { offer ->
                onUndoHandOff(offer)
                viewModel.offerDone(offer)
            }
            onBack()
        }
    }
    val shown by rememberUpdatedState(onShownMessages)
    LaunchedEffect(list) { list?.let { messages -> shown(messages.map { it.id }) } }
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    LaunchedEffect(viewModel) {
        viewModel.notices.collect { snackbar.showSnackbar(resources.getString(it)) }
    }
    val offer = undo.offer
    LaunchedEffect(offer) {
        if (offer != null) {
            if (showSwipeUndo(snackbar, resources, offer)) viewModel.undo(offer.messageId)
            viewModel.offerDone(offer)
        }
    }
    val moveTargets by viewModel.moveTargets.collectAsStateWithLifecycle()
    moveTargets?.let { (_, folders) ->
        MoveDialog(folders = folders, onPick = viewModel::move, onDismiss = viewModel::cancelMove)
    }
    val swipes by viewModel.swipes.collectAsStateWithLifecycle()
    ConversationContent(
        title = title,
        messages = list,
        onBack = onBack,
        onOpenMessage = onOpenMessage,
        swipe = SwipeHandlers(right = swipes.first, left = swipes.second, onSwipe = viewModel::swipe),
        snackbar = snackbar,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationContent(
    title: String,
    messages: List<MessageSummary>?,
    onBack: () -> Unit,
    onOpenMessage: (Long) -> Unit,
    /** Swipes on the messages, as in the list (#552). */
    swipe: SwipeHandlers = SwipeHandlers(),
    snackbar: SnackbarHostState = remember { SnackbarHostState() },
) {
    val locale = LocalConfiguration.current.locales[0]
    val now by currentMinute()
    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painter = painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.thread_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        if (messages == null) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            return@Scaffold
        }
        LazyColumn(Modifier.fillMaxSize().padding(padding).testTag("conversation")) {
            items(messages, key = { it.id }) { message ->
                SwipeableRow(
                    right = swipe.right,
                    left = swipe.left,
                    a11yLabel = { swipeA11yLabel(it, message) },
                    onSwipe = { swipe.onSwipe(message, it) },
                    // Only Delete takes a message out of the conversation (Trash isn't shown). Archived or
                    // moved, it stays in the conversation, so the row springs back at once (#571).
                    removesRow = { it == SwipeAction.DELETE },
                ) {
                    MessageRow(
                        message = message,
                        now = now,
                        locale = locale,
                        modifier = Modifier.clickable { onOpenMessage(message.id) },
                        // Inside a conversation each message shows its time too (#399).
                        dateStyle = RowDateStyle.CONVERSATION,
                    )
                }
                HorizontalDivider(
                    modifier = Modifier.padding(start = 24.dp),
                    color = MaterialTheme.colorScheme.outlineVariant,
                )
            }
        }
    }
}
