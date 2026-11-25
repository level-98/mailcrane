package app.mailcrane.ui.messagelist

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.UNDO_WINDOW_MS
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.components.EmptyFolderState
import app.mailcrane.ui.message.MoveDialog
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.time.ZonedDateTime

/**
 * The messages of folder [folderId], newest first. [onCompose] adds a compose button, which carries
 * [composeFabModifier] (for the shared transition into the compose screen). [notice] is shown once
 * in a snackbar, then [onNoticeShown] is called.
 */
@Composable
fun MessageListScreen(
    folderId: Long,
    title: String,
    onOpenMessage: (Long) -> Unit,
    onCompose: (() -> Unit)? = null,
    composeFabModifier: Modifier = Modifier,
    notice: String? = null,
    onNoticeShown: () -> Unit = {},
    onOpenFolders: (() -> Unit)? = null,
    folders: List<FolderSummary> = emptyList(),
    onOpenSearch: (() -> Unit)? = null,
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
    LaunchedEffect(viewModel) {
        viewModel.notices.collect { snackbar.showSnackbar(resources.getString(it)) }
    }
    LaunchedEffect(viewModel) {
        viewModel.undoOffers.collect { offer ->
            val label = if (offer.action == SwipeAction.DELETE) R.string.swipe_deleted else R.string.swipe_archived
            val result =
                withTimeoutOrNull(UNDO_WINDOW_MS) {
                    snackbar.showSnackbar(
                        message = resources.getString(label),
                        actionLabel = resources.getString(R.string.undo_send_undo),
                        duration = SnackbarDuration.Indefinite,
                    )
                }
            if (result == SnackbarResult.ActionPerformed) viewModel.undo(offer.messageId)
        }
    }
    val swipes by viewModel.swipes.collectAsStateWithLifecycle()
    var moving by rememberSaveable { mutableStateOf<Long?>(null) }
    moving?.let { messageId ->
        MoveDialog(
            folders = folders.filter { it.id != folderId },
            onPick = {
                moving = null
                viewModel.move(messageId, it)
            },
            onDismiss = { moving = null },
        )
    }
    val swipeHandlers =
        SwipeHandlers(
            right = swipes.first,
            left = swipes.second,
            onSwipe = { message, action ->
                if (action == SwipeAction.MOVE) moving = message.id else viewModel.swipe(message, action)
            },
        )
    LaunchedEffect(viewModel) {
        viewModel.actionsUndone.collect { snackbar.showSnackbar(resources.getString(R.string.action_undone)) }
    }
    val noticeShown by rememberUpdatedState(onNoticeShown)
    LaunchedEffect(notice) {
        if (notice != null) {
            snackbar.showSnackbar(notice)
            noticeShown()
        }
    }
    MessageListContent(
        state = state,
        title = title,
        snackbar = snackbar,
        onRefresh = viewModel::refresh,
        onLoadMore = viewModel::loadMore,
        onOpenMessage = onOpenMessage,
        onCompose = onCompose,
        fabModifier = composeFabModifier,
        onOpenFolders = onOpenFolders,
        swipe = swipeHandlers,
        onOpenSearch = onOpenSearch,
    )
}

/** What swiping a row does, and the code that runs it. */
data class SwipeHandlers(
    val right: SwipeAction = SwipeAction.NONE,
    val left: SwipeAction = SwipeAction.NONE,
    val onSwipe: (MessageSummary, SwipeAction) -> Unit = { _, _ -> },
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MessageListContent(
    state: MessageListState,
    title: String,
    snackbar: SnackbarHostState,
    onRefresh: () -> Unit,
    onLoadMore: () -> Unit,
    onOpenMessage: (Long) -> Unit = {},
    onOpenFolders: (() -> Unit)? = null,
    listState: LazyListState = rememberLazyListState(),
    onCompose: (() -> Unit)? = null,
    fabModifier: Modifier = Modifier,
    swipe: SwipeHandlers = SwipeHandlers(),
    onOpenSearch: (() -> Unit)? = null,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                actions = {
                    if (onOpenSearch != null) {
                        IconButton(onClick = onOpenSearch) {
                            Icon(
                                painter = painterResource(R.drawable.ic_mc_search),
                                contentDescription = stringResource(R.string.search_open),
                            )
                        }
                    }
                },
                navigationIcon = {
                    if (onOpenFolders != null) {
                        IconButton(onClick = onOpenFolders) {
                            Icon(
                                painter = painterResource(R.drawable.ic_mc_menu),
                                contentDescription = stringResource(R.string.folders_open),
                            )
                        }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
        floatingActionButton = {
            if (onCompose != null) {
                val label = stringResource(R.string.compose_fab)
                ExtendedFloatingActionButton(
                    onClick = onCompose,
                    icon = { Icon(painterResource(R.drawable.ic_mc_compose), contentDescription = null) },
                    text = { Text(label) },
                    // Collapses to the icon while scrolling down, so it covers less of the list.
                    expanded = !listState.canScrollBackward,
                    modifier = fabModifier,
                )
            }
        },
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
                state.messages.isEmpty() && state.isRefreshing ->
                    Centered {
                        Text(
                            text = stringResource(R.string.message_list_loading),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                state.messages.isEmpty() -> EmptyFolderState()
                else -> Messages(state, listState, onLoadMore, onOpenMessage, swipe)
            }
        }
    }
}

@Composable
private fun Messages(
    state: MessageListState,
    listState: LazyListState,
    onLoadMore: () -> Unit,
    onOpenMessage: (Long) -> Unit,
    swipe: SwipeHandlers,
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
            SwipeableRow(message, swipe) {
                MessageRow(
                    message = message,
                    now = now,
                    locale = locale,
                    modifier = Modifier.clickable { onOpenMessage(message.id) },
                )
            }
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

/**
 * A row that runs the swipe settings' actions, with the same actions in TalkBack's actions menu. The row
 * springs back after a swipe: the list changes when the core says so, never by itself.
 */
@Composable
private fun SwipeableRow(
    message: MessageSummary,
    swipe: SwipeHandlers,
    content: @Composable () -> Unit,
) {
    val state = rememberSwipeToDismissBoxState()
    val scope = rememberCoroutineScope()
    val resources = LocalResources.current
    val actions =
        listOf(swipe.right, swipe.left).distinct().filter { it != SwipeAction.NONE }.map { action ->
            CustomAccessibilityAction(resources.getString(swipeA11yLabel(action, message))) {
                swipe.onSwipe(message, action)
                true
            }
        }
    SwipeToDismissBox(
        state = state,
        modifier = Modifier.semantics { customActions = actions },
        enableDismissFromStartToEnd = swipe.right != SwipeAction.NONE,
        enableDismissFromEndToStart = swipe.left != SwipeAction.NONE,
        onDismiss = { value ->
            val action = if (value == SwipeToDismissBoxValue.StartToEnd) swipe.right else swipe.left
            swipe.onSwipe(message, action)
            scope.launch { state.reset() }
        },
        backgroundContent = {
            val action =
                when (state.dismissDirection) {
                    SwipeToDismissBoxValue.StartToEnd -> swipe.right
                    SwipeToDismissBoxValue.EndToStart -> swipe.left
                    SwipeToDismissBoxValue.Settled -> SwipeAction.NONE
                }
            SwipeBackground(action, state.dismissDirection)
        },
    ) {
        Surface(color = MaterialTheme.colorScheme.surface) { content() }
    }
}

@Composable
private fun SwipeBackground(
    action: SwipeAction,
    direction: SwipeToDismissBoxValue,
) {
    if (action == SwipeAction.NONE) return
    val destructive = action == SwipeAction.DELETE
    val colors = MaterialTheme.colorScheme
    val container = if (destructive) colors.errorContainer else colors.secondaryContainer
    val onContainer = if (destructive) colors.onErrorContainer else colors.onSecondaryContainer
    Box(
        Modifier
            .fillMaxSize()
            .background(container)
            .padding(horizontal = 24.dp),
        contentAlignment =
            if (direction == SwipeToDismissBoxValue.StartToEnd) Alignment.CenterStart else Alignment.CenterEnd,
    ) {
        Icon(
            painter = painterResource(swipeIcon(action)),
            contentDescription = null,
            tint = onContainer,
        )
    }
}

private fun swipeIcon(action: SwipeAction): Int =
    when (action) {
        SwipeAction.ARCHIVE -> R.drawable.ic_mc_archive
        SwipeAction.DELETE -> R.drawable.ic_mc_trash
        SwipeAction.READ_UNREAD -> R.drawable.ic_mc_mail
        SwipeAction.FLAG -> R.drawable.ic_mc_star
        SwipeAction.MOVE, SwipeAction.NONE -> R.drawable.ic_mc_folder
    }

/** What TalkBack offers for [action] on [message], e.g. "Mark as unread" for a read one. */
internal fun swipeA11yLabel(
    action: SwipeAction,
    message: MessageSummary,
): Int =
    when (action) {
        SwipeAction.ARCHIVE -> R.string.message_archive
        SwipeAction.DELETE -> R.string.message_delete
        SwipeAction.READ_UNREAD -> if (message.isSeen) R.string.message_mark_unread else R.string.message_mark_read
        SwipeAction.FLAG -> if (message.isFlagged) R.string.message_unflag else R.string.message_flag
        SwipeAction.MOVE -> R.string.message_move
        SwipeAction.NONE -> R.string.swipe_none
    }
