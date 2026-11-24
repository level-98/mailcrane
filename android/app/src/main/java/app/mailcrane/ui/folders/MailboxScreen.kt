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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.ui.messagelist.MessageListScreen
import app.mailcrane.ui.messagelist.errorText
import kotlinx.coroutines.launch

/** The mail of account [accountId]: the open folder's messages, with the folder drawer beside them. */
@Composable
fun MailboxScreen(
    accountId: Long,
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
                MessageListScreen(
                    folderId = selected.id,
                    title = folderName(selected),
                    onOpenFolders = { scope.launch { drawerState.open() } },
                )
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
