package app.mailcrane.ui.search

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DateRangePicker
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
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
import androidx.compose.material3.rememberDateRangePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
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
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.NO_FILTERS
import app.mailcrane.core.SearchFilters
import app.mailcrane.core.SearchFlag
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.UNIFIED_FOLDER_ID
import app.mailcrane.core.accountColor
import app.mailcrane.settings.AppSettings
import app.mailcrane.ui.components.AccountDot
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
import java.time.LocalDate
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

data class SearchState(
    val query: String = "",
    /** Null before the first search of the current query finishes. */
    val results: List<MessageSummary>? = null,
    /** The server is being asked for matches that were never downloaded. */
    val searchingServer: Boolean = false,
    /** The filters the query uses, shown as chips (M4-21). */
    val filters: SearchFilters = NO_FILTERS,
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

        /** Searching from the Unified Inbox searches every account (M4-21). */
        private val everyAccount = folderId == UNIFIED_FOLDER_ID

        /** Searches locally a moment after typing stops. */
        fun onQueryChange(query: String) {
            _state.update { it.copy(query = query, filters = core.searchFilters(query)) }
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
                // The Unified Inbox is no one folder to ask the server about.
                if (everyAccount) return@launch
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

        /** A chip turned an on/off filter on or off. */
        fun onFlag(
            flag: SearchFlag,
            on: Boolean,
        ) = onQueryChange(core.searchWithFlag(_state.value.query, flag, on))

        /** The From chip: starts a `from:` term to type into, or removes the ones there are. */
        fun onFrom() {
            val query = _state.value.query
            if (_state.value.filters.from.isEmpty()) {
                val start = if (query.isBlank() || query.endsWith(" ")) query else "$query "
                _state.update { it.copy(query = "${start}from:") }
            } else {
                onQueryChange(core.searchWithoutFrom(query))
            }
        }

        /** The Date chip: from [afterDay] to [beforeDay] (exclusive), days since 1970; nulls remove it. */
        fun onDates(
            afterDay: Long?,
            beforeDay: Long?,
        ) = onQueryChange(core.searchWithDates(_state.value.query, afterDay, beforeDay))

        /** The Account chip: only mail of the account with address [email], or every account with null. */
        fun onAccount(email: String?) = onQueryChange(core.searchWithAccount(_state.value.query, email))

        fun useRecent(query: String) {
            _state.update { it.copy(query = query, filters = core.searchFilters(query)) }
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
                    if (everyAccount) {
                        core.searchAllAccounts(query, LIMIT)
                    } else {
                        core.searchMessages(accountId, null, query, LIMIT)
                    }
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
    accounts: List<AccountSummary> = emptyList(),
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
        chips =
            SearchChipActions(
                accounts = accounts,
                onFlag = viewModel::onFlag,
                onFrom = viewModel::onFrom,
                onDates = viewModel::onDates,
                onAccount = viewModel::onAccount,
            ),
    )
}

/** What the search chips (M4-21) can do. [accounts] fills the Account chip, shown with two or more. */
data class SearchChipActions(
    val accounts: List<AccountSummary> = emptyList(),
    val onFlag: (SearchFlag, Boolean) -> Unit = { _, _ -> },
    val onFrom: () -> Unit = {},
    val onDates: (Long?, Long?) -> Unit = { _, _ -> },
    val onAccount: (String?) -> Unit = {},
)

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
    chips: SearchChipActions = SearchChipActions(),
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
            SearchChips(
                filters = state.filters,
                actions =
                    chips.copy(onFrom = {
                        chips.onFrom()
                        focus.requestFocus()
                    }),
            )
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

/** Filter chips under the search field (M4-21). Each one edits the query text, so typed filters show here too. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SearchChips(
    filters: SearchFilters,
    actions: SearchChipActions,
) {
    var pickingDates by remember { mutableStateOf(false) }
    var pickingAccount by remember { mutableStateOf(false) }
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier =
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 4.dp)
                .testTag("search-chips"),
    ) {
        val from = filters.from.joinToString(", ")
        FilterChip(
            selected = from.isNotEmpty(),
            onClick = actions.onFrom,
            label = {
                Text(
                    if (from.isEmpty()) {
                        stringResource(R.string.search_chip_from)
                    } else {
                        stringResource(R.string.search_chip_from_value, from)
                    },
                    maxLines = 1,
                )
            },
            modifier = Modifier.testTag("chip-from"),
        )
        FlagChip(R.string.search_chip_attachment, filters.hasAttachment, "chip-attachment") {
            actions.onFlag(SearchFlag.HAS_ATTACHMENT, it)
        }
        FlagChip(R.string.search_chip_unread, filters.unread, "chip-unread") { actions.onFlag(SearchFlag.UNREAD, it) }
        FlagChip(R.string.search_chip_flagged, filters.flagged, "chip-flagged") { actions.onFlag(SearchFlag.FLAGGED, it) }
        val dated = filters.afterDay != null || filters.beforeDay != null
        FilterChip(
            selected = dated,
            onClick = { pickingDates = true },
            label = { Text(if (dated) dateRangeLabel(filters) else stringResource(R.string.search_chip_date), maxLines = 1) },
            modifier = Modifier.testTag("chip-date"),
        )
        if (actions.accounts.size > 1) {
            val chosen = filters.account
            Box {
                FilterChip(
                    selected = chosen != null,
                    onClick = { pickingAccount = true },
                    label = { Text(chosen ?: stringResource(R.string.search_chip_account), maxLines = 1) },
                    modifier = Modifier.testTag("chip-account"),
                )
                DropdownMenu(expanded = pickingAccount, onDismissRequest = { pickingAccount = false }) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.search_chip_all_accounts)) },
                        onClick = {
                            pickingAccount = false
                            actions.onAccount(null)
                        },
                    )
                    actions.accounts.forEach { account ->
                        DropdownMenuItem(
                            leadingIcon = { AccountDot(accountColor(account.id).toInt()) },
                            text = { Text(account.email, maxLines = 1) },
                            onClick = {
                                pickingAccount = false
                                actions.onAccount(account.email)
                            },
                        )
                    }
                }
            }
        }
    }
    if (pickingDates) {
        DateRangeDialog(
            filters = filters,
            onDismiss = { pickingDates = false },
            onChoose = { after, before ->
                pickingDates = false
                actions.onDates(after, before)
            },
        )
    }
}

@Composable
private fun FlagChip(
    label: Int,
    selected: Boolean,
    tag: String,
    onChange: (Boolean) -> Unit,
) {
    FilterChip(
        selected = selected,
        onClick = { onChange(!selected) },
        label = { Text(stringResource(label), maxLines = 1) },
        modifier = Modifier.testTag(tag),
    )
}

/** The chosen days, e.g. "1 Nov 2026 - 30 Nov 2026" (the query's `before:` day is the one after). */
@Composable
private fun dateRangeLabel(filters: SearchFilters): String {
    val format = remember { DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM) }
    val start = filters.afterDay?.let { LocalDate.ofEpochDay(it).format(format) }
    val end = filters.beforeDay?.let { LocalDate.ofEpochDay(it - 1).format(format) }
    return when {
        start != null && end != null -> stringResource(R.string.search_chip_date_range, start, end)
        start != null -> stringResource(R.string.search_chip_date_from, start)
        else -> stringResource(R.string.search_chip_date_until, end.orEmpty())
    }
}

private const val MILLIS_PER_DAY = 86_400_000L

/** Picks the first and last day to search; "Clear" removes the dates. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DateRangeDialog(
    filters: SearchFilters,
    onDismiss: () -> Unit,
    onChoose: (Long?, Long?) -> Unit,
) {
    val picker =
        rememberDateRangePickerState(
            initialSelectedStartDateMillis = filters.afterDay?.let { it * MILLIS_PER_DAY },
            initialSelectedEndDateMillis = filters.beforeDay?.let { (it - 1) * MILLIS_PER_DAY },
        )
    DatePickerDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton(
                enabled = picker.selectedStartDateMillis != null,
                onClick = {
                    // Picker dates are midnight UTC; the last day is included, so search before the day after it.
                    val start = picker.selectedStartDateMillis?.let { Math.floorDiv(it, MILLIS_PER_DAY) }
                    val end = (picker.selectedEndDateMillis ?: picker.selectedStartDateMillis)?.let { Math.floorDiv(it, MILLIS_PER_DAY) + 1 }
                    onChoose(start, end)
                },
            ) { Text(stringResource(R.string.search_date_ok)) }
        },
        dismissButton = {
            Row {
                if (filters.afterDay != null || filters.beforeDay != null) {
                    TextButton(onClick = { onChoose(null, null) }) { Text(stringResource(R.string.search_date_clear)) }
                }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.search_date_cancel)) }
            }
        },
    ) {
        DateRangePicker(
            state = picker,
            title = { Text(stringResource(R.string.search_date_title), modifier = Modifier.padding(start = 24.dp, top = 16.dp)) },
            modifier = Modifier.weight(1f),
        )
    }
}
