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

        private val _undoOffers = Channel<UndoOffer>(Channel.BUFFERED)

        /** "Archived · Undo" and "Deleted · Undo" after a swipe. */
        val undoOffers: Flow<UndoOffer> = _undoOffers.receiveAsFlow()

        private val _notices = Channel<Int>(Channel.BUFFERED)

        /** One-off messages from swipes, as string resources. */
        val notices: Flow<Int> = _notices.receiveAsFlow()

        private val _moveTargets = MutableStateFlow<Pair<Long, List<FolderSummary>>?>(null)

        /** While a Move swipe's dialog is open: the message, and the folders of its account to offer. */
        val moveTargets: StateFlow<Pair<Long, List<FolderSummary>>?> = _moveTargets.asStateFlow()

        /** Runs [action] on [message] through the core, as a swipe in the list does. Move asks for a folder. */
        fun swipe(
            message: MessageSummary,
            action: SwipeAction,
        ) {
            if (action == SwipeAction.MOVE) return startMove(message)
            val (coreAction, undoable) = swipeCoreAction(message, action) ?: return
            run(message.id, coreAction, undoable, action)
        }

        private fun startMove(message: MessageSummary) {
            viewModelScope.launch {
                try {
                    // The conversation spans folders, so every folder of the message's account is offered.
                    _moveTargets.value = message.id to core.listFolders(message.accountId).filter { it.takesMoves }
                } catch (_: SyncFailure) {
                    _notices.send(R.string.action_not_applied)
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
            viewModelScope.launch {
                val refused = runSwipeAction(core, messageId, action, undoable)
                when {
                    refused != null -> _notices.send(refused)
                    undoable -> _undoOffers.send(UndoOffer(messageId, swipe))
                }
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

@Composable
fun ConversationScreen(
    threadId: Long,
    title: String,
    onBack: () -> Unit,
    onOpenMessage: (Long) -> Unit,
    /** The message ids shown, in order, for "After deleting or moving" (M4-15). */
    onShownMessages: (List<Long>) -> Unit = {},
    viewModel: ConversationViewModel =
        hiltViewModel<ConversationViewModel, ConversationViewModel.Factory>(
            key = "thread-$threadId",
            creationCallback = { factory -> factory.create(threadId) },
        ),
) {
    val messages by viewModel.messages.collectAsStateWithLifecycle()
    val list = messages
    // A conversation that became empty (all moved away) closes.
    LaunchedEffect(list) { if (list != null && list.isEmpty()) onBack() }
    val shown by rememberUpdatedState(onShownMessages)
    LaunchedEffect(list) { list?.let { messages -> shown(messages.map { it.id }) } }
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    LaunchedEffect(viewModel) {
        viewModel.notices.collect { snackbar.showSnackbar(resources.getString(it)) }
    }
    LaunchedEffect(viewModel) {
        viewModel.undoOffers.collect { offer ->
            if (showSwipeUndo(snackbar, resources, offer)) viewModel.undo(offer.messageId)
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
