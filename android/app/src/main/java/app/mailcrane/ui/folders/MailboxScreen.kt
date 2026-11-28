package app.mailcrane.ui.folders

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.PermanentNavigationDrawer
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.FolderRole
import app.mailcrane.core.UNIFIED_FOLDER_ID
import app.mailcrane.settings.RuleDraft
import app.mailcrane.settings.RulesScreen
import app.mailcrane.settings.SettingsScreen
import app.mailcrane.ui.MailLayout
import app.mailcrane.ui.MailPanes
import app.mailcrane.ui.NoMessageSelected
import app.mailcrane.ui.compose.ComposePrefill
import app.mailcrane.ui.compose.DraftRequest
import app.mailcrane.ui.compose.LocalDraftsScreen
import app.mailcrane.ui.compose.OutboxScreen
import app.mailcrane.ui.compose.OutboxViewModel
import app.mailcrane.ui.compose.ReplyRequest
import app.mailcrane.ui.conversation.ConversationScreen
import app.mailcrane.ui.mailLayoutFor
import app.mailcrane.ui.mailShortcuts
import app.mailcrane.ui.message.MessageScreen
import app.mailcrane.ui.messagelist.MessageListScreen
import app.mailcrane.ui.messagelist.UndoOffer
import app.mailcrane.ui.messagelist.errorText
import app.mailcrane.ui.search.SearchScreen
import kotlinx.coroutines.launch

/**
 * The mail of account [accountId]: the open folder's messages, with the folder drawer beside them.
 * The compose parameters are passed to [MessageListScreen].
 */
@Composable
fun MailboxScreen(
    accountId: Long,
    accounts: List<app.mailcrane.core.AccountSummary> = emptyList(),
    onSwitchAccount: (Long) -> Unit = {},
    onAddAccount: () -> Unit = {},
    onRemoveAccount: (Long) -> Unit = {},
    onAccountsChanged: () -> Unit = {},
    /** A message to open now (from a widget, M4-11). */
    openMessageId: Long? = null,
    onMessageOpened: () -> Unit = {},
    onCompose: (() -> Unit)? = null,
    composeFabModifier: Modifier = Modifier,
    notice: String? = null,
    onNoticeShown: () -> Unit = {},
    onReply: (ReplyRequest) -> Unit = {},
    onOpenDraft: (DraftRequest) -> Unit = {},
    onComposeTo: (ComposePrefill) -> Unit = {},
    viewModel: FolderDrawerViewModel =
        hiltViewModel<FolderDrawerViewModel, FolderDrawerViewModel.Factory>(
            key = "folders-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val selected = state.selected
    val loadError = state.loadError
    when {
        selected != null -> {
            // The open message is kept here until there is app-wide navigation. The list keeps its
            // scroll position while a message is open.
            var openMessage by rememberSaveable { mutableStateOf<Long?>(null) }
            LaunchedEffect(openMessageId) {
                if (openMessageId != null) {
                    openMessage = openMessageId
                    onMessageOpened()
                }
            }
            val saved = rememberSaveableStateHolder()
            var showSettings by rememberSaveable { mutableStateOf(false) }
            // The Rules screen (M4-09), opened from Settings or from a message.
            var showRules by rememberSaveable { mutableStateOf(false) }
            var ruleFrom by rememberSaveable { mutableStateOf<String?>(null) }
            var showDrafts by rememberSaveable { mutableStateOf(false) }
            var showOutbox by rememberSaveable { mutableStateOf(false) }
            var showManageFolders by rememberSaveable { mutableStateOf(false) }
            var showSearch by rememberSaveable { mutableStateOf(false) }
            // Each tap on search is a new opening; coming back from a result isn't (#481 review).
            var searchOpenings by rememberSaveable { mutableIntStateOf(0) }
            // An open conversation (M4-02): its thread id and title.
            var openThread by rememberSaveable { mutableStateOf<Pair<Long, String>?>(null) }
            // An Undo a conversation handed over as it closed, for the list to show (#571).
            var handedUndo by remember { mutableStateOf<UndoOffer?>(null) }
            if (showManageFolders) {
                BackHandler { showManageFolders = false }
                ManageFoldersScreen(accountId = accountId, onBack = { showManageFolders = false })
                return
            }
            if (showOutbox) {
                BackHandler { showOutbox = false }
                OutboxScreen(
                    accountId = accountId,
                    onEdit = { onOpenDraft(DraftRequest(draftId = it)) },
                    onBack = { showOutbox = false },
                )
                return
            }
            if (showDrafts) {
                BackHandler { showDrafts = false }
                LocalDraftsScreen(
                    accountId = accountId,
                    onOpen = { onOpenDraft(DraftRequest(draftId = it)) },
                    onBack = { showDrafts = false },
                )
                return
            }
            if (showRules) {
                val close = {
                    showRules = false
                    ruleFrom = null
                }
                BackHandler { close() }
                RulesScreen(
                    accountId = accountId,
                    onBack = close,
                    startWith = ruleFrom?.let { RuleDraft.fromSender(it) },
                )
                return
            }
            if (showSettings) {
                BackHandler { showSettings = false }
                SettingsScreen(
                    onBack = { showSettings = false },
                    accountId = accountId,
                    accounts = accounts,
                    onAddAccount = onAddAccount,
                    onRemoveAccount = onRemoveAccount,
                    onAccountsChanged = onAccountsChanged,
                    onOpenRules = { showRules = true },
                )
                return
            }
            val messageId = openMessage
            val layout = mailLayoutFor(LocalConfiguration.current.screenWidthDp.dp)
            // Next and previous message (J/K, ↓/↑) follow the order the list shows.
            var listOrder by remember { mutableStateOf<List<Long>>(emptyList()) }
            // The open conversation's messages, oldest first, for "After deleting or moving" (M4-15).
            var threadOrder by remember { mutableStateOf<List<Long>>(emptyList()) }
            val leaveOrder = if (openThread != null) threadOrder else listOrder
            // Whether the open message came from search, which isn't a list to step through (#372 review).
            var openedFromSearch by remember { mutableStateOf(false) }
            // Its neighbours in the list it was opened from, taken when it opened: by id, and from that list
            // even if another one is shown later (#372 review).
            val neighbours = remember(messageId) { if (openedFromSearch) null else neighboursIn(leaveOrder, messageId) }
            val afterMove: AfterMoveViewModel = hiltViewModel()
            val afterMoveAction by afterMove.action.collectAsStateWithLifecycle()
            val openMessageAt: (Int) -> Unit = { delta ->
                val i = listOrder.indexOf(messageId)
                val next = if (i < 0) listOrder.firstOrNull() else listOrder.getOrNull(i + delta)
                if (next != null) {
                    openedFromSearch = false
                    openMessage = next
                }
            }
            val detail: @Composable () -> Unit = {
                if (messageId != null) {
                    MessageScreen(
                        messageId = messageId,
                        onBack = { openMessage = null },
                        folders = state.folders,
                        onLeft = {
                            val next = messageAfterLeaving(neighbours, afterMoveAction, leaveOrder)
                            // One pane, the lists aren't shown while a message is open, so their orders aren't
                            // refreshed: drop the message from them, or J/K could open it again (#372 review).
                            listOrder = listOrder - messageId
                            threadOrder = threadOrder - messageId
                            openMessage = next
                        },
                        onReply = onReply,
                        onComposeTo = onComposeTo,
                        onCreateRule = {
                            ruleFrom = it
                            showRules = true
                        },
                    )
                } else {
                    NoMessageSelected()
                }
            }
            if (layout == MailLayout.ONE_PANE) {
                if (showSearch && messageId == null) {
                    BackHandler { showSearch = false }
                    SearchScreen(
                        accountId = accountId,
                        folderId = selected.id,
                        onOpenMessage = {
                            openedFromSearch = true
                            openMessage = it
                        },
                        onBack = { showSearch = false },
                        accounts = accounts,
                        opening = searchOpenings,
                    )
                    return
                }
                val thread = openThread
                if (thread != null && messageId == null) {
                    BackHandler { openThread = null }
                    ConversationScreen(
                        threadId = thread.first,
                        title = thread.second,
                        onBack = { openThread = null },
                        onOpenMessage = {
                            openedFromSearch = false
                            openMessage = it
                        },
                        onShownMessages = { threadOrder = it },
                        onUndoHandOff = { handedUndo = it },
                    )
                    return
                }
                if (messageId != null) {
                    BackHandler { openMessage = null }
                    Box(Modifier.mailShortcuts(onCompose, { openMessageAt(1) }, { openMessageAt(-1) })) { detail() }
                    return
                }
            } else {
                // Side by side, Back closes the message first, then search or the conversation.
                BackHandler(enabled = messageId != null) { openMessage = null }
                BackHandler(enabled = messageId == null && (showSearch || openThread != null)) {
                    showSearch = false
                    openThread = null
                }
            }
            val drawerState = rememberDrawerState(DrawerValue.Closed)
            val outbox =
                hiltViewModel<OutboxViewModel, OutboxViewModel.Factory>(
                    key = "outbox-$accountId",
                    creationCallback = { factory -> factory.create(accountId) },
                )
            val outboxEntries by outbox.entries.collectAsStateWithLifecycle()
            // Counted again whenever the drawer opens, so the Outbox entry is current.
            LaunchedEffect(drawerState.isOpen) { outbox.refresh() }
            val scope = rememberCoroutineScope()
            BackHandler(enabled = drawerState.isOpen) { scope.launch { drawerState.close() } }
            val permanentDrawer = layout == MailLayout.THREE_PANES
            val closeDrawer: () -> Unit = { if (!permanentDrawer) scope.launch { drawerState.close() } }
            val drawer: @Composable () -> Unit = {
                FolderDrawer(
                    folders = state.folders,
                    selectedFolderId = selected.id,
                    onSelect = { id ->
                        viewModel.select(id)
                        openMessage = null
                        openThread = null
                        closeDrawer()
                    },
                    onOpenSettings = {
                        closeDrawer()
                        showSettings = true
                    },
                    onOpenLocalDrafts = {
                        closeDrawer()
                        showDrafts = true
                    },
                    outboxCount = outboxEntries.size,
                    accounts = accounts,
                    currentAccountId = accountId,
                    onSwitchAccount = { id ->
                        closeDrawer()
                        onSwitchAccount(id)
                    },
                    unifiedUnread = state.unifiedUnread,
                    onOpenOutbox = {
                        closeDrawer()
                        showOutbox = true
                    },
                    onManageFolders =
                        if (state.canManageFolders) {
                            {
                                closeDrawer()
                                showManageFolders = true
                            }
                        } else {
                            null
                        },
                    permanent = permanentDrawer,
                )
            }
            val list: @Composable () -> Unit = {
                val thread = openThread
                when {
                    layout != MailLayout.ONE_PANE && showSearch ->
                        SearchScreen(
                            accountId = accountId,
                            folderId = selected.id,
                            onOpenMessage = {
                                openedFromSearch = true
                                openMessage = it
                            },
                            onBack = { showSearch = false },
                            accounts = accounts,
                            opening = searchOpenings,
                        )
                    layout != MailLayout.ONE_PANE && thread != null ->
                        ConversationScreen(
                            threadId = thread.first,
                            title = thread.second,
                            onBack = { openThread = null },
                            onOpenMessage = {
                                openedFromSearch = false
                                openMessage = it
                            },
                            onShownMessages = { threadOrder = it },
                            onUndoHandOff = { handedUndo = it },
                        )
                    else ->
                        saved.SaveableStateProvider("folder-${selected.id}") {
                            MessageListScreen(
                                folderId = selected.id,
                                title =
                                    if (selected.id == UNIFIED_FOLDER_ID) {
                                        stringResource(R.string.all_inboxes)
                                    } else {
                                        folderName(selected)
                                    },
                                // A message in the Drafts folder opens for editing, not reading.
                                onOpenMessage = {
                                    if (selected.role == FolderRole.DRAFTS) {
                                        onOpenDraft(DraftRequest(messageId = it))
                                    } else {
                                        openedFromSearch = false
                                        openMessage = it
                                    }
                                },
                                onOpenFolders =
                                    if (permanentDrawer) {
                                        null
                                    } else {
                                        (
                                            {
                                                scope.launch { drawerState.open() }
                                            }
                                        )
                                    },
                                folders = state.folders,
                                onOpenSearch = {
                                    searchOpenings++
                                    showSearch = true
                                },
                                onOpenThread = { id, title -> openThread = id to title },
                                onCompose = onCompose,
                                composeFabModifier = composeFabModifier,
                                notice = notice,
                                onNoticeShown = onNoticeShown,
                                onShownMessages = { listOrder = it },
                                handedUndo = handedUndo,
                                onHandedUndoShown = { handedUndo = null },
                            )
                        }
                }
            }
            val panes: @Composable () -> Unit = { MailPanes(layout, list, detail) }
            Box(Modifier.fillMaxSize().mailShortcuts(onCompose, { openMessageAt(1) }, { openMessageAt(-1) })) {
                if (permanentDrawer) {
                    PermanentNavigationDrawer(drawerContent = drawer) { panes() }
                } else {
                    ModalFolderDrawer(drawerState = drawerState, drawerContent = drawer) { panes() }
                }
            }
        }
        state.isLoading ->
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        loadError != null -> Message(stringResource(errorText(loadError)), onRetry = viewModel::retry)
        else -> Message(stringResource(R.string.folders_none), onRetry = viewModel::retry)
    }
}

@Composable
private fun Message(
    text: String,
    onRetry: () -> Unit,
) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(text)
            TextButton(onClick = onRetry) { Text(stringResource(R.string.message_list_retry)) }
        }
    }
}
