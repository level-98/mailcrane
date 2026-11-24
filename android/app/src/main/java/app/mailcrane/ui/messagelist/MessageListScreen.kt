package app.mailcrane.ui.messagelist

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.SyncErrorKind
import kotlinx.coroutines.delay
import java.time.ZonedDateTime

/** The messages of folder [folderId], newest first. */
@Composable
fun MessageListScreen(
    folderId: Long,
    title: String,
    viewModel: MessageListViewModel =
        hiltViewModel<MessageListViewModel, MessageListViewModel.Factory>(
            key = "folder-$folderId",
            creationCallback = { factory -> factory.create(folderId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    LaunchedEffect(viewModel) {
        viewModel.errors.collect { kind -> snackbar.showSnackbar(resources.getString(errorText(kind))) }
    }
    MessageListContent(
        state = state,
        title = title,
        snackbar = snackbar,
        onRefresh = viewModel::refresh,
        onLoadMore = viewModel::loadMore,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MessageListContent(
    state: MessageListState,
    title: String,
    snackbar: SnackbarHostState,
    onRefresh: () -> Unit,
    onLoadMore: () -> Unit,
    listState: LazyListState = rememberLazyListState(),
) {
    Scaffold(
        topBar = { TopAppBar(title = { Text(title) }) },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = state.isRefreshing,
            onRefresh = onRefresh,
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            when {
                state.isLoading -> Centered { CircularProgressIndicator() }
                state.loadError != null && state.messages.isEmpty() ->
                    Centered {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(stringResource(errorText(state.loadError)))
                            TextButton(onClick = onRefresh) { Text(stringResource(R.string.message_list_retry)) }
                        }
                    }
                state.messages.isEmpty() ->
                    Centered {
                        val text =
                            if (state.isRefreshing) R.string.message_list_loading else R.string.message_list_empty
                        Text(
                            text = stringResource(text),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                else -> Messages(state, listState, onLoadMore)
            }
        }
    }
}

@Composable
private fun Messages(
    state: MessageListState,
    listState: LazyListState,
    onLoadMore: () -> Unit,
) {
    val locale = LocalConfiguration.current.locales[0]
    val now by currentMinute()
    val loadMore by rememberUpdatedState(onLoadMore)
    val nearEnd by remember(listState) {
        derivedStateOf {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull()?.index ?: return@derivedStateOf false
            last >= info.totalItemsCount - 1 - MessageListViewModel.LOAD_MORE_THRESHOLD
        }
    }
    // Asks again after each page arrives, in case the new end is still on screen.
    LaunchedEffect(nearEnd, state.messages.size) {
        if (nearEnd) loadMore()
    }
    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
        items(items = state.messages, key = { it.id }, contentType = { "message" }) { message ->
            MessageRow(message = message, now = now, locale = locale)
            HorizontalDivider(
                modifier = Modifier.padding(start = 24.dp),
                color = MaterialTheme.colorScheme.outlineVariant,
            )
        }
        if (state.isLoadingOlder) {
            item(key = "loading-older", contentType = "progress") {
                Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
            }
        }
    }
}

/** The current time, updated each minute so "today" and times stay right while the screen is open. */
@Composable
private fun currentMinute() =
    produceState(ZonedDateTime.now()) {
        while (true) {
            val seconds = value.second
            delay((60 - seconds) * 1_000L)
            value = ZonedDateTime.now()
        }
    }

/** Fills the space and stays scrollable, so pull-to-refresh also works on an empty list. */
@Composable
private fun Centered(content: @Composable () -> Unit) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        item {
            Box(modifier = Modifier.fillParentMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                content()
            }
        }
    }
}

internal fun errorText(kind: SyncErrorKind): Int =
    when (kind) {
        SyncErrorKind.NETWORK -> R.string.sync_error_network
        SyncErrorKind.TLS -> R.string.sync_error_tls
        SyncErrorKind.AUTH -> R.string.sync_error_auth
        SyncErrorKind.PROTOCOL -> R.string.sync_error_protocol
        SyncErrorKind.TIMEOUT -> R.string.sync_error_timeout
        SyncErrorKind.STORAGE -> R.string.sync_error_storage
        SyncErrorKind.NOT_FOUND -> R.string.sync_error_not_found
        SyncErrorKind.FOLDER_RESET -> R.string.sync_error_folder_reset
    }
