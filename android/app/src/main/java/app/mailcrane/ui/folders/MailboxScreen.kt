package app.mailcrane.ui.folders

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
import app.mailcrane.ui.compose.ComposePrefill
import app.mailcrane.ui.compose.DraftRequest
import app.mailcrane.ui.compose.LocalDraftsScreen
import app.mailcrane.ui.compose.OutboxScreen
import app.mailcrane.ui.compose.OutboxViewModel
import app.mailcrane.ui.compose.ReplyRequest
import app.mailcrane.ui.conversation.ConversationScreen
import app.mailcrane.ui.message.MessageScreen
import app.mailcrane.ui.messagelist.MessageListScreen
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
            val saved = rememberSaveableStateHolder()
            var showSettings by rememberSaveable { mutableStateOf(false) }
            // The Rules screen (M4-09), opened from Settings or from a message.
            var showRules by rememberSaveable { mutableStateOf(false) }
            var ruleFrom by rememberSaveable { mutableStateOf<String?>(null) }
            var showDrafts by rememberSaveable { mutableStateOf(false) }
            var showOutbox by rememberSaveable { mutableStateOf(false) }
            var showSearch by rememberSaveable { mutableStateOf(false) }
            // An open conversation (M4-02): its thread id and title.
            var openThread by rememberSaveable { mutableStateOf<Pair<Long, String>?>(null) }
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
                    onOpenRules = { showRules = true },
                )
                return
            }
            val messageId = openMessage
            if (showSearch && messageId == null) {
                BackHandler { showSearch = false }
                SearchScreen(
                    accountId = accountId,
                    folderId = selected.id,
                    onOpenMessage = { openMessage = it },
                    onBack = { showSearch = false },
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
                    onOpenMessage = { openMessage = it },
                )
                return
            }
            if (messageId != null) {
                BackHandler { openMessage = null }
                MessageScreen(
                    messageId = messageId,
                    onBack = { openMessage = null },
                    folders = state.folders,
                    onReply = onReply,
                    onComposeTo = onComposeTo,
                    onCreateRule = {
                        ruleFrom = it
                        showRules = true
                    },
                )
                return
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
            ModalNavigationDrawer(
                drawerState = drawerState,
                drawerContent = {
                    FolderDrawer(
                        folders = state.folders,
                        selectedFolderId = selected.id,
                        onSelect = { id ->
                            viewModel.select(id)
                            scope.launch { drawerState.close() }
                        },
                        onOpenSettings = {
                            scope.launch { drawerState.close() }
                            showSettings = true
                        },
                        onOpenLocalDrafts = {
                            scope.launch { drawerState.close() }
                            showDrafts = true
                        },
                        outboxCount = outboxEntries.size,
                        accounts = accounts,
                        currentAccountId = accountId,
                        onSwitchAccount = { id ->
                            scope.launch { drawerState.close() }
                            onSwitchAccount(id)
                        },
                        unifiedUnread = state.unifiedUnread,
                        onOpenOutbox = {
                            scope.launch { drawerState.close() }
                            showOutbox = true
                        },
                    )
                },
            ) {
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
                                openMessage = it
                            }
                        },
                        onOpenFolders = { scope.launch { drawerState.open() } },
                        folders = state.folders,
                        onOpenSearch = { showSearch = true },
                        onOpenThread = { id, title -> openThread = id to title },
                        onCompose = onCompose,
                        composeFabModifier = composeFabModifier,
                        notice = notice,
                        onNoticeShown = onNoticeShown,
                    )
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
