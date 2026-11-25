package app.mailcrane.ui.messagelist

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import app.mailcrane.R

/** What tapping and long-pressing rows, and the selection top bar, do (M4-14). */
data class SelectionHandlers(
    val onToggleMessage: (Long) -> Unit = {},
    val onToggleThread: (Long) -> Unit = {},
    val onClear: () -> Unit = {},
    val onAction: (BulkAction) -> Unit = {},
    val onMove: () -> Unit = {},
)

/**
 * The top bar while rows are selected: the count (announced by TalkBack as it changes), Clear, and the
 * actions for every selected row. Read and Flag turn into Unread and Unflag when every selected row
 * already is read or flagged.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SelectionTopBar(
    state: MessageListState,
    handlers: SelectionHandlers,
) {
    val count = state.selectedCount
    val allRead = state.allSelectedRead
    val allFlagged = state.allSelectedFlagged
    TopAppBar(
        modifier = Modifier.testTag("selection-bar"),
        colors =
            TopAppBarDefaults.topAppBarColors(
                containerColor = MaterialTheme.colorScheme.secondaryContainer,
                titleContentColor = MaterialTheme.colorScheme.onSecondaryContainer,
                navigationIconContentColor = MaterialTheme.colorScheme.onSecondaryContainer,
                actionIconContentColor = MaterialTheme.colorScheme.onSecondaryContainer,
            ),
        title = {
            Text(
                text = pluralStringResource(R.plurals.bulk_selected_count, count, count),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
        },
        navigationIcon = {
            IconButton(onClick = handlers.onClear) {
                Icon(painterResource(R.drawable.ic_mc_close), contentDescription = stringResource(R.string.bulk_clear))
            }
        },
        actions = {
            BarAction(
                icon = R.drawable.ic_mc_mail,
                label = if (allRead) R.string.message_mark_unread else R.string.message_mark_read,
            ) { handlers.onAction(if (allRead) BulkAction.UNREAD else BulkAction.READ) }
            BarAction(
                icon = R.drawable.ic_mc_star,
                label = if (allFlagged) R.string.message_unflag else R.string.message_flag,
            ) { handlers.onAction(if (allFlagged) BulkAction.UNFLAG else BulkAction.FLAG) }
            BarAction(R.drawable.ic_mc_archive, R.string.message_archive) { handlers.onAction(BulkAction.ARCHIVE) }
            BarAction(R.drawable.ic_mc_trash, R.string.message_delete) { handlers.onAction(BulkAction.DELETE) }
            BarAction(R.drawable.ic_mc_folder, R.string.message_move, handlers.onMove)
        },
    )
}

@Composable
private fun BarAction(
    icon: Int,
    label: Int,
    onClick: () -> Unit,
) {
    IconButton(onClick = onClick) {
        Icon(painterResource(icon), contentDescription = stringResource(label))
    }
}

/** Asked before deleting selected messages that are already in Trash: that deletes them for good. */
@Composable
internal fun DeleteForGoodDialog(
    count: Int,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(pluralStringResource(R.plurals.bulk_delete_forever_title, count, count)) },
        text = { Text(stringResource(R.string.bulk_delete_forever_text)) },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(stringResource(R.string.bulk_delete_forever_confirm), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.message_move_cancel)) } },
    )
}

/** The bulk snackbar's text, e.g. "3 archived". */
internal fun bulkUndoLabel(kind: BulkUndoKind): Int =
    when (kind) {
        BulkUndoKind.ARCHIVED -> R.plurals.bulk_archived
        BulkUndoKind.DELETED -> R.plurals.bulk_deleted
        BulkUndoKind.DELETED_FOR_GOOD -> R.plurals.bulk_deleted_for_good
        BulkUndoKind.MOVED -> R.plurals.bulk_moved
    }
