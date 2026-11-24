package app.mailcrane.ui.messagelist

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.ui.message.MessageScreen
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

sealed interface InboxState {
    data object Loading : InboxState

    data class Ready(
        val folderId: Long,
    ) : InboxState

    data object NoInbox : InboxState

    data class Failed(
        val kind: SyncErrorKind,
    ) : InboxState
}

/** Finds the account's inbox by refreshing its folder list. Until the folder drawer (M1-13). */
@HiltViewModel(assistedFactory = InboxViewModel.Factory::class)
class InboxViewModel
    @AssistedInject
    constructor(
        private val core: MailboxCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): InboxViewModel
        }

        private val _state = MutableStateFlow<InboxState>(InboxState.Loading)
        val state: StateFlow<InboxState> = _state.asStateFlow()

        init {
            retry()
        }

        fun retry() {
            _state.value = InboxState.Loading
            viewModelScope.launch {
                _state.value =
                    try {
                        core.refreshFolders(accountId)?.let { InboxState.Ready(it) } ?: InboxState.NoInbox
                    } catch (e: SyncFailure.Failed) {
                        InboxState.Failed(e.kind)
                    }
            }
        }
    }

/** The inbox of account [accountId]. */
@Composable
fun InboxScreen(
    accountId: Long,
    viewModel: InboxViewModel =
        hiltViewModel<InboxViewModel, InboxViewModel.Factory>(
            key = "inbox-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    when (val s = state) {
        is InboxState.Ready -> {
            // Until navigation arrives with the folder drawer, the open message is kept here. The
            // list keeps its scroll position while a message is open.
            var openMessage by rememberSaveable { mutableStateOf<Long?>(null) }
            val saved = rememberSaveableStateHolder()
            val messageId = openMessage
            if (messageId != null) {
                BackHandler { openMessage = null }
                MessageScreen(messageId = messageId, onBack = { openMessage = null })
            } else {
                saved.SaveableStateProvider("folder-${s.folderId}") {
                    MessageListScreen(
                        folderId = s.folderId,
                        title = stringResource(R.string.message_list_inbox),
                        onOpenMessage = { openMessage = it },
                    )
                }
            }
        }
        InboxState.Loading ->
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        InboxState.NoInbox -> Message(stringResource(R.string.message_list_no_inbox), onRetry = null)
        is InboxState.Failed -> Message(stringResource(errorText(s.kind)), onRetry = viewModel::retry)
    }
}

@Composable
private fun Message(
    text: String,
    onRetry: (() -> Unit)?,
) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(text)
            if (onRetry != null) {
                TextButton(onClick = onRetry) { Text(stringResource(R.string.message_list_retry)) }
            }
        }
    }
}
