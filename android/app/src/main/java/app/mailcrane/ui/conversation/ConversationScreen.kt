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
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
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
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncFailure
import app.mailcrane.ui.messagelist.MessageRow
import app.mailcrane.ui.messagelist.currentMinute
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** The messages of one conversation, oldest first, across folders (M4-02). */
@HiltViewModel(assistedFactory = ConversationViewModel.Factory::class)
class ConversationViewModel
    @AssistedInject
    constructor(
        private val core: MailboxCore,
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
    ConversationContent(title, list, onBack, onOpenMessage)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationContent(
    title: String,
    messages: List<MessageSummary>?,
    onBack: () -> Unit,
    onOpenMessage: (Long) -> Unit,
) {
    val locale = LocalConfiguration.current.locales[0]
    val now by currentMinute()
    Scaffold(
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
                MessageRow(
                    message = message,
                    now = now,
                    locale = locale,
                    modifier = Modifier.clickable { onOpenMessage(message.id) },
                )
                HorizontalDivider(
                    modifier = Modifier.padding(start = 24.dp),
                    color = MaterialTheme.colorScheme.outlineVariant,
                )
            }
        }
    }
}
