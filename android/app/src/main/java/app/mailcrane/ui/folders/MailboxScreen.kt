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
import app.mailcrane.ui.message.MessageScreen
import app.mailcrane.ui.messagelist.MessageListScreen
import app.mailcrane.ui.messagelist.errorText
import kotlinx.coroutines.launch

/**
 * The mail of account [accountId]: the open folder's messages, with the folder drawer beside them.
 * The compose parameters are passed to [MessageListScreen].
 */
@Composable
fun MailboxScreen(
    accountId: Long,
    onCompose: (() -> Unit)? = null,
    composeFabModifier: Modifier = Modifier,
    notice: String? = null,
    onNoticeShown: () -> Unit = {},
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
            val messageId = openMessage
            if (messageId != null) {
                BackHandler { openMessage = null }
                MessageScreen(messageId = messageId, onBack = { openMessage = null })
                return
            }
            val drawerState = rememberDrawerState(DrawerValue.Closed)
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
                    )
                },
            ) {
                saved.SaveableStateProvider("folder-${selected.id}") {
                    MessageListScreen(
                        folderId = selected.id,
                        title = folderName(selected),
                        onOpenMessage = { openMessage = it },
                        onOpenFolders = { scope.launch { drawerState.open() } },
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
