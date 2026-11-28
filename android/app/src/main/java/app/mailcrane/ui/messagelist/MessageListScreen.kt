package app.mailcrane.ui.messagelist

import android.content.res.Resources
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
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
import androidx.compose.material3.SwipeToDismissBoxDefaults
import androidx.compose.material3.SwipeToDismissBoxState
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.ThreadSummary
import app.mailcrane.core.UNDO_WINDOW_MS
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.components.EmptyFolderState
import app.mailcrane.ui.folders.takesMoves
import app.mailcrane.ui.message.MoveDialog
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.time.ZonedDateTime
import java.util.Locale

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
    onOpenThread: (Long, String) -> Unit = { _, _ -> },
    /** The ids shown, in order, for next/previous message (M4-13). */
    onShownMessages: (List<Long>) -> Unit = {},
    viewModel: MessageListViewModel =
        hiltViewModel<MessageListViewModel, MessageListViewModel.Factory>(
            key = "folder-$folderId",
            creationCallback = { factory -> factory.create(folderId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val shown by rememberUpdatedState(onShownMessages)
    LaunchedEffect(state.messages) { shown(state.messages.map { it.id }) }
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
            if (showSwipeUndo(snackbar, resources, offer)) viewModel.undo(offer.messageId)
        }
    }
    val swipes by viewModel.swipes.collectAsStateWithLifecycle()
    var moving by rememberSaveable { mutableStateOf<Long?>(null) }
    moving?.let { messageId ->
        MoveDialog(
            folders = folders.filter { it.id != folderId && it.takesMoves },
            onPick = {
                moving = null
                viewModel.move(messageId, it)
            },
            onDismiss = { moving = null },
        )
    }
    var movingThread by rememberSaveable { mutableStateOf<Long?>(null) }
    movingThread?.let { threadId ->
        MoveDialog(
            folders = folders.filter { it.id != folderId && it.takesMoves },
            onPick = {
                movingThread = null
                viewModel.moveThread(threadId, it)
            },
            onDismiss = { movingThread = null },
        )
    }
    val swipeHandlers =
        SwipeHandlers(
            right = swipes.first,
            left = swipes.second,
            onSwipe = { message, action ->
                if (action == SwipeAction.MOVE) moving = message.id else viewModel.swipe(message, action)
            },
            onSwipeThread = { thread, action ->
                if (action == SwipeAction.MOVE) {
                    movingThread = thread.threadId
                } else {
                    viewModel.swipeThread(thread, action)
                }
            },
        )
    LaunchedEffect(viewModel) {
        viewModel.actionsUndone.collect { message -> snackbar.showSnackbar(resources.getString(message)) }
    }
    // Multi-select (M4-14).
    LaunchedEffect(viewModel) {
        viewModel.bulkUndoOffers.collect { offer ->
            val count = offer.messageIds.size
            val text = resources.getQuantityString(bulkUndoLabel(offer.kind), count, count)
            // A permanent delete is said, with no Undo it couldn't keep.
            if (offer.kind == BulkUndoKind.DELETED_FOR_GOOD) {
                snackbar.showSnackbar(text)
                return@collect
            }
            val result =
                withTimeoutOrNull(UNDO_WINDOW_MS) {
                    snackbar.showSnackbar(
                        message = text,
                        actionLabel = resources.getString(R.string.undo_send_undo),
                        duration = SnackbarDuration.Indefinite,
                    )
                }
            if (result == SnackbarResult.ActionPerformed) viewModel.undoBulk(offer.messageIds)
        }
    }
    val moveTargets by viewModel.moveTargets.collectAsStateWithLifecycle()
    moveTargets?.let { targets ->
        MoveDialog(folders = targets, onPick = viewModel::bulkMove, onDismiss = viewModel::cancelMove)
    }
    // A Delete that would be permanent (in Trash, or an account without one) asks first.
    if (state.confirmDeleteForGood > 0) {
        DeleteForGoodDialog(
            count = state.confirmDeleteForGood,
            onConfirm = viewModel::confirmDeleteForGood,
            onDismiss = viewModel::cancelDeleteForGood,
        )
    }
    val selection =
        SelectionHandlers(
            onToggleMessage = viewModel::toggleMessage,
            onToggleThread = viewModel::toggleThread,
            onClear = viewModel::clearSelection,
            onAction = viewModel::bulk,
            onMove = { viewModel.startMove(folders) },
        )
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
        onOpenThread = onOpenThread,
        selection = selection,
    )
}

/** What swiping a row does, and the code that runs it: on a message row, or on a conversation row (#552). */
data class SwipeHandlers(
    val right: SwipeAction = SwipeAction.NONE,
    val left: SwipeAction = SwipeAction.NONE,
    val onSwipe: (MessageSummary, SwipeAction) -> Unit = { _, _ -> },
    val onSwipeThread: (ThreadSummary, SwipeAction) -> Unit = { _, _ -> },
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
    onOpenThread: (Long, String) -> Unit = { _, _ -> },
    listState: LazyListState = rememberLazyListState(),
    onCompose: (() -> Unit)? = null,
    fabModifier: Modifier = Modifier,
    swipe: SwipeHandlers = SwipeHandlers(),
    onOpenSearch: (() -> Unit)? = null,
    selection: SelectionHandlers = SelectionHandlers(),
) {
    // Back ends selection before it leaves the screen (M4-14).
    BackHandler(enabled = state.selecting) { selection.onClear() }
    val fabShown = onCompose != null && !state.selecting
    // The compose button's height, so the list's end can scroll clear of it at any font scale (#464);
    // the Scaffold already keeps both above the navigation bar. Saved, so a list restored at its end
    // (Back, another folder, rotation) has the space on its first frame; until the first measure,
    // the button's minimum height, which it keeps up to the largest font scale Android offers.
    var fabHeightPx by rememberSaveable { mutableIntStateOf(0) }
    // Seen inside a lookahead scope (the shared transition into Compose): only its pass counts.
    var lookahead by remember { mutableStateOf(false) }
    val fabHeight = with(LocalDensity.current) { if (fabHeightPx > 0) fabHeightPx.toDp() else FAB_MIN_HEIGHT }
    // Kept while selecting, so the rows don't jump under the finger when a long-press starts it.
    val listEndPadding = if (onCompose != null) fabHeight + FAB_MARGIN else 0.dp
    Scaffold(
        topBar = {
            if (state.selecting) {
                SelectionTopBar(state, selection)
                return@Scaffold
            }
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
            if (onCompose != null && fabShown) {
                val label = stringResource(R.string.compose_fab)
                ExtendedFloatingActionButton(
                    onClick = onCompose,
                    icon = { Icon(painterResource(R.drawable.ic_mc_compose), contentDescription = null) },
                    text = { Text(label) },
                    // Collapses to the icon while scrolling down, so it covers less of the list.
                    expanded = !listState.canScrollBackward,
                    // One label whether expanded or not: collapsed, the icon alone said nothing (M6-01).
                    modifier =
                        Modifier
                            // Outside the shared bounds, and in a transition only from the lookahead
                            // pass, which has the final size: the animated frames don't count (#471 review).
                            .layout { measurable, constraints ->
                                val placeable = measurable.measure(constraints)
                                if (isLookingAhead) lookahead = true
                                if (isLookingAhead || !lookahead) fabHeightPx = placeable.height
                                layout(placeable.width, placeable.height) { placeable.place(0, 0) }
                            }.then(fabModifier)
                            .clearAndSetSemantics {
                                contentDescription = label
                                role = Role.Button
                                onClick {
                                    onCompose()
                                    true
                                }
                            },
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
                else ->
                    Messages(
                        state = state,
                        listState = listState,
                        onLoadMore = onLoadMore,
                        onOpenMessage = onOpenMessage,
                        // No swipes while selecting: a stray swipe would act on one row only.
                        swipe = if (state.selecting) SwipeHandlers() else swipe,
                        onOpenThread = onOpenThread,
                        selection = selection,
                        endPadding = listEndPadding,
                    )
            }
        }
    }
}

/** The space Material 3's Scaffold leaves between a floating action button and the content's edge. */
private val FAB_MARGIN = 16.dp

/** Material 3's minimum floating action button height, used until the button has been measured. */
private val FAB_MIN_HEIGHT = 56.dp

@Composable
private fun Messages(
    state: MessageListState,
    listState: LazyListState,
    onLoadMore: () -> Unit,
    onOpenMessage: (Long) -> Unit,
    swipe: SwipeHandlers,
    onOpenThread: (Long, String) -> Unit = { _, _ -> },
    selection: SelectionHandlers = SelectionHandlers(),
    /** Space after the last row, so it can scroll clear of the compose button (#464). */
    endPadding: Dp = 0.dp,
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
    val byId = remember(state.messages) { state.messages.associateBy { it.id } }
    val today = now.toLocalDate()
    val showThreads = state.grouped && state.threads.isNotEmpty()
    val weekStart = firstDayOfWeek(locale)
    val threadEntries =
        remember(state.threads, showThreads, today, now.zone, weekStart) {
            if (showThreads) {
                withDateHeaders(
                    state.threads,
                    { it.lastDate },
                    today,
                    now.zone,
                    weekStart,
                )
            } else {
                emptyList()
            }
        }
    val messageEntries =
        remember(state.messages, showThreads, today, now.zone, weekStart) {
            if (showThreads) emptyList() else withDateHeaders(state.messages, { it.date }, today, now.zone, weekStart)
        }
    LazyColumn(
        state = listState,
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = endPadding),
    ) {
        if (showThreads) {
            for (entry in threadEntries) {
                when (entry) {
                    is ListEntry.Header ->
                        stickyHeader(key = entry.key, contentType = "header") {
                            DateHeader(entry.group, today, locale)
                        }
                    is ListEntry.Row ->
                        item(key = "t${entry.item.threadId}", contentType = "thread") {
                            // Rows that leave or arrive move the others smoothly (#341).
                            Box(Modifier.animateItem()) {
                                ThreadItem(
                                    entry.item,
                                    byId,
                                    now,
                                    locale,
                                    swipe,
                                    onOpenMessage,
                                    onOpenThread,
                                    state,
                                    selection,
                                )
                            }
                        }
                }
            }
        } else {
            for (entry in messageEntries) {
                when (entry) {
                    is ListEntry.Header ->
                        stickyHeader(key = entry.key, contentType = "header") {
                            DateHeader(entry.group, today, locale)
                        }
                    is ListEntry.Row -> {
                        val message = entry.item
                        item(key = message.id, contentType = "message") {
                            // Rows that leave or arrive move the others smoothly (#341).
                            Column(Modifier.animateItem()) {
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
                                        modifier = Modifier.selectable(message.id, state, selection, onOpenMessage),
                                        selecting = state.selecting,
                                        selected = message.id in state.selectedMessages,
                                        onAvatarClick = { selection.onToggleMessage(message.id) },
                                    )
                                }
                                RowDivider()
                            }
                        }
                    }
                }
            }
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

/** One conversation: a plain message row when it holds one message, else a thread row (M4-02). */
@Composable
private fun ThreadItem(
    thread: ThreadSummary,
    byId: Map<Long, MessageSummary>,
    now: ZonedDateTime,
    locale: Locale,
    swipe: SwipeHandlers,
    onOpenMessage: (Long) -> Unit,
    onOpenThread: (Long, String) -> Unit,
    state: MessageListState,
    selection: SelectionHandlers,
) {
    val single = byId[thread.newestMessageId]?.takeIf { thread.messageCount <= 1u }
    if (single != null) {
        SwipeableRow(
            right = swipe.right,
            left = swipe.left,
            a11yLabel = { swipeA11yLabel(it, single) },
            onSwipe = { swipe.onSwipe(single, it) },
        ) {
            MessageRow(
                message = single,
                now = now,
                locale = locale,
                modifier = Modifier.selectable(single.id, state, selection, onOpenMessage),
                selecting = state.selecting,
                selected = single.id in state.selectedMessages,
                onAvatarClick = { selection.onToggleMessage(single.id) },
            )
        }
    } else {
        SwipeableRow(
            right = swipe.right,
            left = swipe.left,
            a11yLabel = { threadSwipeA11yLabel(it, thread) },
            onSwipe = { swipe.onSwipeThread(thread, it) },
        ) {
            ThreadRow(
                thread = thread,
                newest = byId[thread.newestMessageId],
                now = now,
                locale = locale,
                onClick = {
                    when {
                        state.selecting -> selection.onToggleThread(thread.threadId)
                        thread.messageCount <= 1u -> onOpenMessage(thread.newestMessageId)
                        else -> onOpenThread(thread.threadId, thread.subject)
                    }
                },
                onLongClick = { selection.onToggleThread(thread.threadId) },
                selecting = state.selecting,
                selected = thread.threadId in state.selectedThreads,
                onAvatarClick = { selection.onToggleThread(thread.threadId) },
            )
        }
    }
    RowDivider()
}

/** A message row's taps: open it, or while selecting toggle it; a long-press toggles it (M4-14). */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Modifier.selectable(
    messageId: Long,
    state: MessageListState,
    selection: SelectionHandlers,
    onOpenMessage: (Long) -> Unit,
): Modifier =
    combinedClickable(
        onClick = { if (state.selecting) selection.onToggleMessage(messageId) else onOpenMessage(messageId) },
        onLongClick = { selection.onToggleMessage(messageId) },
        onLongClickLabel = stringResource(R.string.message_select),
    )

@Composable
private fun RowDivider() {
    HorizontalDivider(
        modifier = Modifier.padding(start = 24.dp),
        color = MaterialTheme.colorScheme.outlineVariant,
    )
}

/** The current time, updated each minute so "today" and times stay right while the screen is open. */
@Composable
internal fun currentMinute() =
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
        SyncErrorKind.BUSY -> R.string.sync_error_busy
    }

/** Shows "Archived · Undo" or "Deleted · Undo" for [offer] for [UNDO_WINDOW_MS]; true if Undo was tapped. */
internal suspend fun showSwipeUndo(
    snackbar: SnackbarHostState,
    resources: Resources,
    offer: UndoOffer,
): Boolean {
    val label = if (offer.action == SwipeAction.DELETE) R.string.swipe_deleted else R.string.swipe_archived
    val result =
        withTimeoutOrNull(UNDO_WINDOW_MS) {
            snackbar.showSnackbar(
                message = resources.getString(label),
                actionLabel = resources.getString(R.string.undo_send_undo),
                duration = SnackbarDuration.Indefinite,
            )
        }
    return result == SnackbarResult.ActionPerformed
}

/** Whether swiping [action] takes the message out of the list it's in (archive, delete). */
internal fun swipeRemovesRow(action: SwipeAction): Boolean =
    action == SwipeAction.ARCHIVE || action == SwipeAction.DELETE

/** How long a row swiped away waits for the list to drop it before it comes back: the action didn't take. */
internal const val SWIPE_REMOVE_TIMEOUT_MS = 3_000L

/**
 * A row that runs the swipe settings' actions, with the same actions in TalkBack's actions menu. The list
 * changes when the core says so, never by itself. A row swiped to archive or delete stays swiped away until
 * then, and the rows below close the gap ([animateItem]); it used to spring back first and then vanish in
 * one frame (#341). If the list keeps it (the action failed), it comes back after [SWIPE_REMOVE_TIMEOUT_MS].
 * Other actions (read, flag, move) spring back at once: the row stays.
 */
@Composable
internal fun SwipeableRow(
    right: SwipeAction,
    left: SwipeAction,
    /** TalkBack's label for an action on this row. */
    a11yLabel: (SwipeAction) -> Int,
    onSwipe: (SwipeAction) -> Unit,
    /**
     * For Settings' sample row (#553): when set, a swipe reports its direction (`true` for start to end)
     * instead of running [onSwipe], the row always springs straight back, and TalkBack gets one action per
     * direction labelled by [directionLabel], even when both directions have the same action.
     */
    onSwipeToward: ((startToEnd: Boolean) -> Unit)? = null,
    directionLabel: ((startToEnd: Boolean) -> String)? = null,
    content: @Composable () -> Unit,
) {
    // Not saved: material3's dismiss effect runs onDismiss whenever the state isn't Settled, so a row
    // restored mid-swipe (after rotation or process death) would run its action a second time (#466).
    // A row coming back always starts at rest.
    val threshold = SwipeToDismissBoxDefaults.positionalThreshold
    val state = remember { SwipeToDismissBoxState(SwipeToDismissBoxValue.Settled, threshold) }
    val scope = rememberCoroutineScope()
    val resources = LocalResources.current
    val actions =
        if (onSwipeToward != null) {
            // The same condition as a swipe's (onDismiss below): with onSwipeToward set, TalkBack
            // never runs a mail action either (#594 review).
            listOf(true to right, false to left)
                .filter { it.second != SwipeAction.NONE }
                .map { (startToEnd, action) ->
                    val label = directionLabel?.invoke(startToEnd) ?: resources.getString(a11yLabel(action))
                    CustomAccessibilityAction(label) {
                        onSwipeToward(startToEnd)
                        true
                    }
                }
        } else {
            listOf(right, left).distinct().filter { it != SwipeAction.NONE }.map { action ->
                CustomAccessibilityAction(resources.getString(a11yLabel(action))) {
                    onSwipe(action)
                    true
                }
            }
        }
    // SwipeToDismissBox keys its dismiss effect on this callback as well as on the settled value, so a
    // new lambda on every recomposition runs the action again. A read or flag swipe changes the row,
    // the row recomposes while it is still springing back, and the action ran again: the message
    // flipped read, unread, read… and each run restarted the spring from rest, so the row crawled back
    // over seconds (#368). One callback for the row's lifetime, reading the latest actions and handler
    // (which holds the latest message or conversation).
    val currentRight by rememberUpdatedState(right)
    val currentLeft by rememberUpdatedState(left)
    val currentOnSwipe by rememberUpdatedState(onSwipe)
    val currentOnSwipeToward by rememberUpdatedState(onSwipeToward)
    val onDismiss: (SwipeToDismissBoxValue) -> Unit =
        remember(state, scope) {
            { value ->
                val startToEnd = value == SwipeToDismissBoxValue.StartToEnd
                val action = if (startToEnd) currentRight else currentLeft
                val toward = currentOnSwipeToward
                if (toward != null) toward(startToEnd) else currentOnSwipe(action)
                scope.launch {
                    if (toward == null && swipeRemovesRow(action)) delay(SWIPE_REMOVE_TIMEOUT_MS)
                    state.reset()
                }
            }
        }
    SwipeToDismissBox(
        state = state,
        modifier = Modifier.semantics { customActions = actions },
        enableDismissFromStartToEnd = right != SwipeAction.NONE,
        enableDismissFromEndToStart = left != SwipeAction.NONE,
        // Only a row at rest takes a swipe. material3 1.5.0 already disables dragging until the state is
        // Settled; this only says so here, in case a later version stops doing it: a swipe caught mid-reset
        // would settle at the same value, run nothing, and leave the row swiped away for good (#368, #369 review).
        gesturesEnabled = state.settledValue == SwipeToDismissBoxValue.Settled,
        onDismiss = onDismiss,
        backgroundContent = {
            val action =
                when (state.dismissDirection) {
                    SwipeToDismissBoxValue.StartToEnd -> right
                    SwipeToDismissBoxValue.EndToStart -> left
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
            modifier = Modifier.testTag(swipeIconTag(action)),
        )
    }
}

/** The tag of a swipe background's icon, for tests. */
internal fun swipeIconTag(action: SwipeAction): String = "swipe-icon-${action.name}"

private fun swipeIcon(action: SwipeAction): Int =
    when (action) {
        SwipeAction.ARCHIVE -> R.drawable.ic_mc_archive
        SwipeAction.DELETE -> R.drawable.ic_mc_trash
        SwipeAction.READ_UNREAD -> R.drawable.ic_mc_mail
        SwipeAction.FLAG -> R.drawable.ic_mc_star
        SwipeAction.MOVE, SwipeAction.NONE -> R.drawable.ic_mc_folder
    }

/**
 * What TalkBack offers for [action] on a conversation row: read and flag go by the row, as the swipe
 * does (#552).
 */
internal fun threadSwipeA11yLabel(
    action: SwipeAction,
    thread: ThreadSummary,
): Int =
    when (action) {
        SwipeAction.ARCHIVE -> R.string.message_archive
        SwipeAction.DELETE -> R.string.message_delete
        SwipeAction.READ_UNREAD ->
            if (thread.unreadCount > 0u) R.string.message_mark_read else R.string.message_mark_unread
        SwipeAction.FLAG -> if (thread.isFlagged) R.string.message_unflag else R.string.message_flag
        SwipeAction.MOVE -> R.string.message_move
        SwipeAction.NONE -> R.string.swipe_none
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
