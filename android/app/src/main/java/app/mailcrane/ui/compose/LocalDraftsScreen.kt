package app.mailcrane.ui.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.DraftSummary
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** The drafts saved on this device for one account. */
@HiltViewModel(assistedFactory = LocalDraftsViewModel.Factory::class)
class LocalDraftsViewModel
    @AssistedInject
    constructor(
        private val core: ComposeCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): LocalDraftsViewModel
        }

        private val _drafts = MutableStateFlow<List<DraftSummary>?>(null)

        /** Null while loading. */
        val drafts: StateFlow<List<DraftSummary>?> = _drafts.asStateFlow()

        fun refresh() {
            viewModelScope.launch {
                _drafts.value =
                    try {
                        core.listDrafts(accountId)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        emptyList()
                    }
            }
        }
    }

/** The account's drafts saved on this device, including ones not uploaded yet. */
@Composable
fun LocalDraftsScreen(
    accountId: Long,
    onOpen: (Long) -> Unit,
    onBack: () -> Unit,
    viewModel: LocalDraftsViewModel =
        hiltViewModel<LocalDraftsViewModel, LocalDraftsViewModel.Factory>(
            key = "drafts-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    LaunchedEffect(viewModel) { viewModel.refresh() }
    val drafts by viewModel.drafts.collectAsStateWithLifecycle()
    LocalDraftsContent(drafts = drafts, onOpen = onOpen, onBack = onBack)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LocalDraftsContent(
    drafts: List<DraftSummary>?,
    onOpen: (Long) -> Unit,
    onBack: () -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.local_drafts_title)) },
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
        when {
            drafts == null -> Unit
            drafts.isEmpty() ->
                Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.local_drafts_empty))
                }
            else ->
                LazyColumn(Modifier.fillMaxSize().padding(padding)) {
                    items(items = drafts, key = { it.id }) { draft ->
                        DraftRow(draft) { onOpen(draft.id) }
                    }
                }
        }
    }
}

@Composable
private fun DraftRow(
    draft: DraftSummary,
    onClick: () -> Unit,
) {
    val status =
        when {
            draft.isConflict -> R.string.local_drafts_conflict
            draft.onServer -> R.string.local_drafts_on_server
            else -> R.string.local_drafts_not_uploaded
        }
    ListItem(
        overlineContent = {
            Text(
                text = draft.to.ifBlank { stringResource(R.string.local_drafts_no_recipients) },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = {
            Text(
                stringResource(status),
                color =
                    if (draft.isConflict) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
            )
        },
        modifier = Modifier.clickable(role = Role.Button, onClick = onClick).padding(horizontal = 4.dp),
    ) {
        Text(
            text = draft.subject.ifBlank { stringResource(R.string.local_drafts_no_subject) },
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
