package app.mailcrane.ui.search

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncFailure
import app.mailcrane.settings.AppSettings
import app.mailcrane.ui.components.NoResultsState
import app.mailcrane.ui.messagelist.MessageRow
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.time.ZonedDateTime

data class SearchState(
    val query: String = "",
    /** Null before the first search of the current query finishes. */
    val results: List<MessageSummary>? = null,
    /** The server is being asked for matches that were never downloaded. */
    val searchingServer: Boolean = false,
)

/** Search: local results as you type, then the server's when you press search. */
@HiltViewModel(assistedFactory = SearchViewModel.Factory::class)
class SearchViewModel
    @AssistedInject
    constructor(
        private val core: MailboxCore,
        private val settings: AppSettings,
        @Assisted("account") private val accountId: Long,
        @Assisted("folder") private val folderId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(
                @Assisted("account") accountId: Long,
                @Assisted("folder") folderId: Long,
            ): SearchViewModel
        }

        private val _state = MutableStateFlow(SearchState())
        val state: StateFlow<SearchState> = _state.asStateFlow()

        val recent: StateFlow<List<String>> =
            settings.recentSearches.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

        private var typing: Job? = null

        /** Searches locally a moment after typing stops. */
        fun onQueryChange(query: String) {
            _state.update { it.copy(query = query) }
            typing?.cancel()
            typing =
                viewModelScope.launch {
                    delay(TYPING_PAUSE_MS)
                    searchLocal(query)
                }
        }

        /** The search key: local results now, then the server's, and remembers the query. */
        fun submit() {
            val query = _state.value.query.trim()
            if (query.isEmpty()) return
            typing?.cancel()
            viewModelScope.launch {
                settings.addRecentSearch(query)
                searchLocal(query)
                _state.update { it.copy(searchingServer = true) }
                val stored =
                    try {
                        core.searchServer(folderId, query)
                    } catch (_: SyncFailure) {
                        // Offline or refused: the local results stand.
                        0
                    }
                _state.update { it.copy(searchingServer = false) }
                if (stored > 0) searchLocal(query)
            }
        }

        fun useRecent(query: String) {
            _state.update { it.copy(query = query) }
            submit()
        }

        fun clearRecent() {
            viewModelScope.launch { settings.clearRecentSearches() }
        }

        private suspend fun searchLocal(query: String) {
            if (query.isBlank()) {
                _state.update { it.copy(results = null) }
                return
            }
            val found =
                try {
                    core.searchMessages(accountId, null, query, LIMIT)
                } catch (_: SyncFailure) {
                    emptyList()
                }
            // Only if the query hasn't changed meanwhile.
            _state.update { if (it.query.trim() == query.trim()) it.copy(results = found) else it }
        }

        companion object {
            const val TYPING_PAUSE_MS = 250L
            const val LIMIT = 200
        }
    }

@Composable
fun SearchScreen(
    accountId: Long,
    folderId: Long,
    onOpenMessage: (Long) -> Unit,
    onBack: () -> Unit,
    viewModel: SearchViewModel =
        hiltViewModel<SearchViewModel, SearchViewModel.Factory>(
            key = "search-$accountId",
            creationCallback = { factory -> factory.create(accountId, folderId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val recent by viewModel.recent.collectAsStateWithLifecycle()
    SearchContent(
        state = state,
        recent = recent,
        onQueryChange = viewModel::onQueryChange,
        onSubmit = viewModel::submit,
        onRecent = viewModel::useRecent,
        onClearRecent = viewModel::clearRecent,
        onOpenMessage = onOpenMessage,
        onBack = onBack,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SearchContent(
    state: SearchState,
    recent: List<String>,
    onQueryChange: (String) -> Unit,
    onSubmit: () -> Unit,
    onRecent: (String) -> Unit,
    onClearRecent: () -> Unit,
    onOpenMessage: (Long) -> Unit,
    onBack: () -> Unit,
) {
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painter = painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.settings_back),
                        )
                    }
                },
                title = {
                    TextField(
                        value = state.query,
                        onValueChange = onQueryChange,
                        placeholder = { Text(stringResource(R.string.search_hint)) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                        keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
                        colors =
                            TextFieldDefaults.colors(
                                focusedContainerColor = Color.Transparent,
                                unfocusedContainerColor = Color.Transparent,
                            ),
                        modifier = Modifier.fillMaxWidth().focusRequester(focus),
                    )
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            if (state.searchingServer) LinearProgressIndicator(Modifier.fillMaxWidth())
            val results = state.results
            when {
                state.query.isBlank() -> RecentSearches(recent, onRecent, onClearRecent)
                results == null -> Unit
                results.isEmpty() && !state.searchingServer -> Box(Modifier.fillMaxSize()) { NoResultsState() }
                else -> Results(results, onOpenMessage)
            }
        }
    }
}

@Composable
private fun RecentSearches(
    recent: List<String>,
    onRecent: (String) -> Unit,
    onClear: () -> Unit,
) {
    if (recent.isEmpty()) return
    LazyColumn {
        item {
            ListItem(
                trailingContent = {
                    TextButton(onClick = onClear) { Text(stringResource(R.string.search_clear_recent)) }
                },
            ) {
                Text(stringResource(R.string.search_recent), style = MaterialTheme.typography.titleSmall)
            }
        }
        items(recent) { query ->
            ListItem(
                leadingContent = { Icon(painterResource(R.drawable.ic_mc_search), contentDescription = null) },
                modifier = Modifier.clickable(role = Role.Button) { onRecent(query) },
            ) {
                Text(query)
            }
        }
    }
}

@Composable
private fun Results(
    results: List<MessageSummary>,
    onOpenMessage: (Long) -> Unit,
) {
    val locale = LocalConfiguration.current.locales[0]
    val now = remember { ZonedDateTime.now() }
    LazyColumn(Modifier.fillMaxSize()) {
        items(results, key = { it.id }) { message ->
            MessageRow(
                message = message,
                now = now,
                locale = locale,
                modifier = Modifier.clickable { onOpenMessage(message.id) },
            )
            HorizontalDivider(Modifier.padding(start = 24.dp), color = MaterialTheme.colorScheme.outlineVariant)
        }
    }
}
