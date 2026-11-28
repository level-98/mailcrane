package app.mailcrane.ui.folders

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.FolderSummary

/** Indent per folder level, as in the drawer. */
private val LEVEL_INDENT = 16.dp

/** Create, rename, move and delete the account's folders (M4-22). */
@Composable
fun ManageFoldersScreen(
    accountId: Long,
    onBack: () -> Unit,
    accountEmail: String? = null,
    viewModel: ManageFoldersViewModel =
        hiltViewModel<ManageFoldersViewModel, ManageFoldersViewModel.Factory>(
            key = "manage-folders-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    ManageFoldersContent(
        state = state,
        onBack = onBack,
        onCreate = viewModel::startCreate,
        onRename = viewModel::startRename,
        onMove = viewModel::startMove,
        onDelete = viewModel::startDelete,
        onDismiss = viewModel::dismiss,
        onConfirmCreate = viewModel::confirmCreate,
        onConfirmRename = viewModel::confirmRename,
        onConfirmMove = viewModel::confirmMove,
        onConfirmDelete = viewModel::confirmDelete,
        accountEmail = accountEmail,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ManageFoldersContent(
    state: ManageFoldersState,
    onBack: () -> Unit,
    onCreate: (FolderSummary?) -> Unit,
    onRename: (FolderSummary) -> Unit,
    onMove: (FolderSummary) -> Unit,
    onDelete: (FolderSummary) -> Unit,
    onDismiss: () -> Unit,
    onConfirmCreate: (String) -> Unit,
    onConfirmRename: (String) -> Unit,
    onConfirmMove: (Long?) -> Unit,
    onConfirmDelete: () -> Unit,
    /** The account whose folders these are, shown under the title (#396). */
    accountEmail: String? = null,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column(Modifier.semantics(mergeDescendants = true) { heading() }) {
                        Text(stringResource(R.string.folders_manage_title))
                        if (!accountEmail.isNullOrBlank()) {
                            Text(
                                accountEmail,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.folders_back),
                        )
                    }
                },
                actions = {
                    if (state.canManage) {
                        TextButton(onClick = { onCreate(null) }, modifier = Modifier.testTag("new-folder")) {
                            Text(stringResource(R.string.folders_new))
                        }
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            if (state.loaded && !state.canManage) {
                Text(
                    text = stringResource(R.string.folders_error_unsupported),
                    modifier = Modifier.padding(16.dp),
                )
            }
            LazyColumn(Modifier.fillMaxSize().testTag("manage-folders")) {
                items(items = state.folders, key = { it.id }) { folder ->
                    FolderRow(
                        folder = folder,
                        canManage = state.canManage,
                        onCreate = onCreate,
                        onRename = onRename,
                        onMove = onMove,
                        onDelete = onDelete,
                    )
                }
            }
        }
    }
    when (val dialog = state.dialog) {
        null -> {}
        is FolderDialog.Create ->
            NameDialog(
                title =
                    dialog.parent?.let { stringResource(R.string.folders_new_in, folderName(it)) }
                        ?: stringResource(R.string.folders_new),
                initial = "",
                confirm = stringResource(R.string.folders_create),
                state = state,
                onConfirm = onConfirmCreate,
                onDismiss = onDismiss,
            )
        is FolderDialog.Rename ->
            NameDialog(
                title = stringResource(R.string.folders_rename_title, folderName(dialog.folder)),
                initial = dialog.folder.displayName,
                confirm = stringResource(R.string.folders_rename),
                state = state,
                onConfirm = onConfirmRename,
                onDismiss = onDismiss,
            )
        is FolderDialog.Move -> MoveDialog(dialog, state, onConfirmMove, onDismiss)
        is FolderDialog.Delete -> DeleteDialog(dialog.folder, state, onConfirmDelete, onDismiss)
    }
}

@Composable
private fun FolderRow(
    folder: FolderSummary,
    canManage: Boolean,
    onCreate: (FolderSummary?) -> Unit,
    onRename: (FolderSummary) -> Unit,
    onMove: (FolderSummary) -> Unit,
    onDelete: (FolderSummary) -> Unit,
) {
    var menu by rememberSaveable { mutableStateOf(false) }
    val name = folderName(folder)
    ListItem(
        headlineContent = { Text(name) },
        leadingContent = { Icon(painterResource(folderIcon(folder.role)), contentDescription = null) },
        trailingContent = {
            if (canManage && (folder.canHoldSubfolders || folder.canChange)) {
                Box {
                    IconButton(onClick = { menu = true }, modifier = Modifier.testTag("folder-menu-${folder.id}")) {
                        Icon(
                            painterResource(R.drawable.ic_mc_more),
                            contentDescription = stringResource(R.string.folders_actions_for, name),
                        )
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        val item = @Composable { text: Int, action: () -> Unit ->
                            DropdownMenuItem(text = { Text(stringResource(text)) }, onClick = {
                                menu = false
                                action()
                            })
                        }
                        if (folder.canHoldSubfolders) {
                            item(R.string.folders_new_subfolder) { onCreate(folder) }
                        }
                        if (folder.canChange) {
                            item(R.string.folders_rename) { onRename(folder) }
                            item(R.string.folders_move) { onMove(folder) }
                            item(R.string.folders_delete) { onDelete(folder) }
                        }
                    }
                }
            }
        },
        modifier = Modifier.padding(start = LEVEL_INDENT * folder.depth.toInt().coerceAtMost(MAX_LEVELS)),
    )
}

/** Deeper levels aren't indented further, as in the drawer. */
private const val MAX_LEVELS = 6

@Composable
private fun ErrorText(state: ManageFoldersState) {
    val error = state.error ?: return
    Text(
        text = if (error.arg != null) stringResource(error.text, error.arg) else stringResource(error.text),
        color = MaterialTheme.colorScheme.error,
        style = MaterialTheme.typography.bodyMedium,
        modifier = Modifier.padding(top = 8.dp).semantics { liveRegion = LiveRegionMode.Polite },
    )
}

@Composable
private fun NameDialog(
    title: String,
    initial: String,
    confirm: String,
    state: ManageFoldersState,
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var name by rememberSaveable { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text(stringResource(R.string.folders_name)) },
                    singleLine = true,
                    isError = state.error != null,
                    enabled = !state.busy,
                    keyboardOptions =
                        KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { onConfirm(name) }),
                    modifier = Modifier.fillMaxWidth().testTag("folder-name"),
                )
                ErrorText(state)
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(name) }, enabled = !state.busy && name.isNotBlank()) { Text(confirm) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !state.busy) { Text(stringResource(R.string.folders_cancel)) }
        },
    )
}

@Composable
private fun MoveDialog(
    dialog: FolderDialog.Move,
    state: ManageFoldersState,
    onConfirm: (Long?) -> Unit,
    onDismiss: () -> Unit,
) {
    val targets = dialog.targets
    // -1 means "nothing chosen yet"; null means the top level.
    var chosen by rememberSaveable { mutableStateOf<Long?>(-1L) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.folders_move_title, folderName(dialog.folder))) },
        text = {
            Column(Modifier.heightIn(max = 400.dp).verticalScroll(rememberScrollState()).selectableGroup()) {
                if (targets == null) {
                    Text(stringResource(R.string.folders_loading))
                } else if (!targets.toTop && targets.folders.isEmpty()) {
                    Text(stringResource(R.string.folders_move_nowhere))
                } else {
                    if (targets.toTop) {
                        Choice(stringResource(R.string.folders_top_level), 0, chosen == null) { chosen = null }
                    }
                    targets.folders.forEach { f ->
                        Choice(folderName(f), f.depth.toInt(), chosen == f.id) { chosen = f.id }
                    }
                }
                ErrorText(state)
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(chosen) }, enabled = !state.busy && chosen != -1L) {
                Text(stringResource(R.string.folders_move))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !state.busy) { Text(stringResource(R.string.folders_cancel)) }
        },
    )
}

@Composable
private fun Choice(
    label: String,
    depth: Int,
    selected: Boolean,
    onClick: () -> Unit,
) {
    ListItem(
        headlineContent = { Text(label) },
        leadingContent = { RadioButton(selected = selected, onClick = null) },
        modifier =
            Modifier
                .selectable(selected = selected, role = Role.RadioButton, onClick = onClick)
                .padding(start = LEVEL_INDENT * depth.coerceAtMost(MAX_LEVELS)),
    )
}

@Composable
private fun DeleteDialog(
    folder: FolderSummary,
    state: ManageFoldersState,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.folders_delete_title, folderName(folder))) },
        text = {
            Column {
                Text(stringResource(R.string.folders_delete_explain))
                ErrorText(state)
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = !state.busy, modifier = Modifier.testTag("confirm-delete")) {
                Text(stringResource(R.string.folders_delete), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !state.busy) { Text(stringResource(R.string.folders_cancel)) }
        },
    )
}
