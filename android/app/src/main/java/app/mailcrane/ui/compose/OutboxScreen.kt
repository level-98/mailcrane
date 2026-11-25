package app.mailcrane.ui.compose

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.OutboxEntry
import app.mailcrane.core.OutboxEntryState
import app.mailcrane.core.ScheduledBy
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** The outbox of one account: unsent messages, with Retry, Edit and Delete. */
@HiltViewModel(assistedFactory = OutboxViewModel.Factory::class)
class OutboxViewModel
    @AssistedInject
    constructor(
        private val core: ComposeCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): OutboxViewModel
        }

        private val _entries = MutableStateFlow<List<OutboxEntry>>(emptyList())
        val entries: StateFlow<List<OutboxEntry>> = _entries.asStateFlow()

        fun refresh() {
            viewModelScope.launch { _entries.value = quietly(emptyList()) { core.listOutbox(accountId) } }
        }

        fun retry(id: Long) = act { core.retryOutbox(id) }

        fun delete(id: Long) = act { core.deleteOutbox(id) }

        /** Takes the message out of the outbox as a draft; [onDraft] gets the draft id. */
        fun edit(
            id: Long,
            onDraft: (Long) -> Unit,
        ) {
            viewModelScope.launch {
                val draft = quietly(null) { core.editOutbox(id) }
                refresh()
                if (draft != null) onDraft(draft)
            }
        }

        private fun act(block: suspend () -> Unit) {
            viewModelScope.launch {
                quietly(Unit) { block() }
                refresh()
            }
        }

        private suspend fun <T> quietly(
            fallback: T,
            block: suspend () -> T,
        ): T =
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Busy (being sent right now) or gone; the refreshed list shows what's true.
                fallback
            }
    }

@Composable
fun OutboxScreen(
    accountId: Long,
    onEdit: (Long) -> Unit,
    onBack: () -> Unit,
    viewModel: OutboxViewModel =
        hiltViewModel<OutboxViewModel, OutboxViewModel.Factory>(
            key = "outbox-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    LaunchedEffect(viewModel) { viewModel.refresh() }
    val entries by viewModel.entries.collectAsStateWithLifecycle()
    OutboxContent(
        entries = entries,
        onRetry = viewModel::retry,
        onEdit = { viewModel.edit(it, onEdit) },
        onDelete = viewModel::delete,
        onBack = onBack,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OutboxContent(
    entries: List<OutboxEntry>,
    onRetry: (Long) -> Unit,
    onEdit: (Long) -> Unit,
    onDelete: (Long) -> Unit,
    onBack: () -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.outbox_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painter = painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.settings_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        if (entries.isEmpty()) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                Text(stringResource(R.string.outbox_empty))
            }
        } else {
            LazyColumn(Modifier.fillMaxSize().padding(padding)) {
                items(items = entries, key = { it.id }) { entry ->
                    OutboxRow(entry, onRetry, onEdit, onDelete)
                    HorizontalDivider()
                }
            }
        }
    }
}

@Composable
private fun OutboxRow(
    entry: OutboxEntry,
    onRetry: (Long) -> Unit,
    onEdit: (Long) -> Unit,
    onDelete: (Long) -> Unit,
) {
    Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(
            text = entry.subject.ifBlank { stringResource(R.string.local_drafts_no_subject) },
            style = MaterialTheme.typography.titleMedium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(entry.to, maxLines = 1, overflow = TextOverflow.Ellipsis)
        entry.sendAt?.let { at ->
            Text(
                text = stringResource(R.string.outbox_scheduled_for, formatSendAt(at)),
                style = MaterialTheme.typography.bodySmall,
            )
        }
        Text(
            text = stringResource(outboxStatus(entry)),
            color =
                if (entry.state == OutboxEntryState.FAILED) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            style = MaterialTheme.typography.bodySmall,
        )
        if (entry.state != OutboxEntryState.SENDING && entry.state != OutboxEntryState.HELD_BY_SERVER) {
            Row {
                if (entry.state == OutboxEntryState.FAILED || entry.state == OutboxEntryState.NOT_SAVED) {
                    TextButton(onClick = { onRetry(entry.id) }) { Text(stringResource(R.string.outbox_retry)) }
                }
                // A sent message can't be edited; it's only its copy that's missing.
                if (entry.state != OutboxEntryState.NOT_SAVED) {
                    TextButton(onClick = { onEdit(entry.id) }) { Text(stringResource(R.string.outbox_edit)) }
                }
                TextButton(onClick = { onDelete(entry.id) }) { Text(stringResource(R.string.outbox_delete)) }
            }
        }
    }
}

/** Why the message is still here, in words. */
internal fun outboxStatus(entry: OutboxEntry): Int =
    when (entry.state) {
        OutboxEntryState.SENDING -> R.string.outbox_sending
        OutboxEntryState.NOT_SAVED -> R.string.outbox_not_saved
        OutboxEntryState.HELD_BY_SERVER -> R.string.outbox_held_by_server
        OutboxEntryState.QUEUED ->
            when {
                entry.scheduledBy == ScheduledBy.DEVICE -> R.string.outbox_scheduled_device
                entry.scheduledBy == ScheduledBy.UNDECIDED -> R.string.outbox_scheduled_undecided
                entry.attempts == 0u -> R.string.outbox_waiting
                else -> R.string.outbox_will_retry
            }
        OutboxEntryState.FAILED ->
            when (entry.lastError) {
                "auth", "missing_credentials" -> R.string.outbox_failed_auth
                "recipients_rejected" -> R.string.outbox_failed_recipients
                "too_large" -> R.string.outbox_failed_too_large
                else -> R.string.outbox_failed
            }
    }
