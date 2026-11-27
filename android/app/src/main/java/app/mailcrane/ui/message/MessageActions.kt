package app.mailcrane.ui.message

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import app.mailcrane.R
import app.mailcrane.core.FolderSummary
import app.mailcrane.ui.folders.folderIcon
import app.mailcrane.ui.folders.folderName

/** What the message toolbar's buttons do. */
data class MessageActions(
    val onArchive: () -> Unit = {},
    val onDelete: () -> Unit = {},
    val onMarkUnread: () -> Unit = {},
    val onToggleFlag: () -> Unit = {},
    val onMove: (Long) -> Unit = {},
    /** Opens the snooze picker; null hides the button. */
    val onSnooze: (() -> Unit)? = null,
    /** "Create rule from this message"; null hides it. */
    val onCreateRule: (() -> Unit)? = null,
)

/**
 * The toolbar actions for an open message: archive, delete, mark unread, flag, and "Move to…" in
 * the overflow menu. [moveTargets] are the folders the message can move to.
 */
@Composable
fun RowScope.MessageToolbarActions(
    flagged: Boolean,
    moveTargets: List<FolderSummary>,
    actions: MessageActions,
    snoozeEnabled: Boolean = true,
) {
    var menuOpen by rememberSaveable { mutableStateOf(false) }
    var picking by rememberSaveable { mutableStateOf(false) }
    IconButton(onClick = actions.onArchive) {
        Icon(painterResource(R.drawable.ic_mc_archive), contentDescription = stringResource(R.string.message_archive))
    }
    IconButton(onClick = actions.onDelete) {
        Icon(painterResource(R.drawable.ic_mc_trash), contentDescription = stringResource(R.string.message_delete))
    }
    IconButton(onClick = actions.onMarkUnread) {
        Icon(
            painter = painterResource(R.drawable.ic_mc_mail),
            contentDescription = stringResource(R.string.message_mark_unread),
        )
    }
    actions.onSnooze?.let { snooze ->
        IconButton(onClick = snooze, enabled = snoozeEnabled) {
            Icon(painterResource(R.drawable.ic_mc_snooze), contentDescription = stringResource(R.string.message_snooze))
        }
    }
    IconButton(onClick = actions.onToggleFlag) {
        Icon(
            painter = painterResource(R.drawable.ic_mc_star),
            contentDescription = stringResource(if (flagged) R.string.message_unflag else R.string.message_flag),
            // Flagged is shown by colour and announced by the label, which changes with it.
            tint = if (flagged) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    if (moveTargets.isNotEmpty()) {
        IconButton(onClick = { menuOpen = true }) {
            Icon(painterResource(R.drawable.ic_mc_more), contentDescription = stringResource(R.string.message_more))
        }
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.message_move)) },
                leadingIcon = { Icon(painterResource(R.drawable.ic_mc_folder), contentDescription = null) },
                onClick = {
                    menuOpen = false
                    picking = true
                },
            )
            actions.onCreateRule?.let { create ->
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.rules_create_from_message)) },
                    onClick = {
                        menuOpen = false
                        create()
                    },
                )
            }
        }
    }
    if (picking) {
        MoveDialog(
            folders = moveTargets,
            onPick = {
                picking = false
                actions.onMove(it)
            },
            onDismiss = { picking = false },
        )
    }
}

/** Picks the folder to move the message to. */
@Composable
internal fun MoveDialog(
    folders: List<FolderSummary>,
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.message_move_title)) },
        text = {
            LazyColumn {
                items(items = folders, key = { it.id }) { folder ->
                    ListItem(
                        leadingContent = { Icon(painterResource(folderIcon(folder.role)), contentDescription = null) },
                        modifier =
                            Modifier
                                .clickable { onPick(folder.id) }
                                .semantics { role = Role.Button },
                    ) {
                        Text(folderName(folder))
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.message_move_cancel)) } },
    )
}
