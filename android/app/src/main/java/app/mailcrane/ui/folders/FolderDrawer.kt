package app.mailcrane.ui.folders

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.NavigationDrawerItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.UNIFIED_FOLDER_ID
import app.mailcrane.ui.components.AccountDot

/** Indent per nesting level. */
private val INDENT_STEP = 16.dp

/** Deeper folders are indented no further, so their names stay readable on a phone. */
private const val MAX_INDENT_LEVELS = 4

/** Unread counts above this show as "999+". */
private const val MAX_BADGE_COUNT = 999u

/** The drawer's content: the account's folders, in the core's order, with [selectedFolderId] highlighted. */
@Composable
fun FolderDrawer(
    folders: List<FolderSummary>,
    selectedFolderId: Long?,
    onSelect: (Long) -> Unit,
    modifier: Modifier = Modifier,
    onOpenSettings: (() -> Unit)? = null,
    onOpenLocalDrafts: (() -> Unit)? = null,
    outboxCount: Int = 0,
    onOpenOutbox: (() -> Unit)? = null,
    /** Opens "Manage folders" (M4-22); null hides the item. */
    onManageFolders: (() -> Unit)? = null,
    accounts: List<AccountSummary> = emptyList(),
    currentAccountId: Long? = null,
    onSwitchAccount: (Long) -> Unit = {},
    unifiedUnread: Int = 0,
    /** Always visible beside the list (wide windows, M4-13) rather than sliding in. */
    permanent: Boolean = false,
) {
    val sheet: @Composable (@Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) -> Unit =
        if (permanent) {
            { content ->
                androidx.compose.material3.PermanentDrawerSheet(modifier = modifier.width(300.dp), content = content)
            }
        } else {
            { content -> ModalDrawerSheet(modifier = modifier, content = content) }
        }
    sheet {
        // With several accounts (M4-01): switch between them, or see every inbox together.
        if (accounts.size > 1) {
            Text(
                text = stringResource(R.string.accounts_title),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 28.dp, vertical = 12.dp).semantics { heading() },
            )
            accounts.forEach { account ->
                NavigationDrawerItem(
                    label = { Text(account.email, maxLines = 1) },
                    icon = { AccountDot(account.color.toInt()) },
                    selected = account.id == currentAccountId && selectedFolderId != UNIFIED_FOLDER_ID,
                    onClick = { onSwitchAccount(account.id) },
                    modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding),
                )
            }
            NavigationDrawerItem(
                label = { Text(stringResource(R.string.all_inboxes)) },
                icon = { Icon(painterResource(R.drawable.ic_mc_inbox), contentDescription = null) },
                badge = { if (unifiedUnread > 0) Text(unifiedUnread.toString()) },
                selected = selectedFolderId == UNIFIED_FOLDER_ID,
                onClick = { onSelect(UNIFIED_FOLDER_ID) },
                modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding),
            )
            HorizontalDivider(Modifier.padding(horizontal = 28.dp, vertical = 8.dp))
        }
        Text(
            text = stringResource(R.string.folders_title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 28.dp, vertical = 16.dp).semantics { heading() },
        )
        LazyColumn(Modifier.weight(1f)) {
            items(items = folders, key = { it.id }) { folder ->
                FolderItem(
                    folder = folder,
                    selected = folder.id == selectedFolderId,
                    onClick = { onSelect(folder.id) },
                )
            }
        }
        if (onOpenSettings != null || onOpenLocalDrafts != null) {
            HorizontalDivider(Modifier.padding(horizontal = 28.dp))
        }
        if (onOpenOutbox != null && outboxCount > 0) {
            NavigationDrawerItem(
                label = { Text(stringResource(R.string.outbox_title)) },
                icon = { Icon(painterResource(R.drawable.ic_mc_sent), contentDescription = null) },
                badge = { Text(outboxCount.toString()) },
                selected = false,
                onClick = onOpenOutbox,
                modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding),
            )
        }
        if (onManageFolders != null) {
            NavigationDrawerItem(
                label = { Text(stringResource(R.string.folders_manage)) },
                icon = { Icon(painterResource(R.drawable.ic_mc_folder), contentDescription = null) },
                selected = false,
                onClick = onManageFolders,
                modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding).testTag("manage-folders-item"),
            )
        }
        if (onOpenLocalDrafts != null) {
            NavigationDrawerItem(
                label = { Text(stringResource(R.string.local_drafts_title)) },
                icon = { Icon(painterResource(R.drawable.ic_mc_drafts), contentDescription = null) },
                selected = false,
                onClick = onOpenLocalDrafts,
                modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding),
            )
        }
        if (onOpenSettings != null) {
            NavigationDrawerItem(
                label = { Text(stringResource(R.string.settings_title)) },
                icon = { Icon(painterResource(R.drawable.ic_mc_settings), contentDescription = null) },
                selected = false,
                onClick = onOpenSettings,
                modifier = Modifier.padding(NavigationDrawerItemDefaults.ItemPadding).padding(vertical = 8.dp),
            )
        }
    }
}

@Composable
private fun FolderItem(
    folder: FolderSummary,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val name = folderName(folder)
    val unread = folder.unreadCount.coerceAtMost(Int.MAX_VALUE.toUInt()).toInt()
    // TalkBack reads one line, e.g. "Inbox, 3 unread"; the visible label and badge stay silent.
    val description =
        if (unread > 0) pluralStringResource(R.plurals.folder_a11y_unread, unread, name, unread) else name
    val indent = INDENT_STEP * folder.depth.coerceAtMost(MAX_INDENT_LEVELS.toUInt()).toInt()
    NavigationDrawerItem(
        label = {
            Text(
                text = name,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.clearAndSetSemantics {},
            )
        },
        icon = { Icon(painterResource(folderIcon(folder.role)), contentDescription = null) },
        badge =
            if (folder.unreadCount > 0u) {
                { Text(unreadBadge(folder.unreadCount), modifier = Modifier.clearAndSetSemantics {}) }
            } else {
                null
            },
        selected = selected,
        onClick = onClick,
        modifier =
            Modifier
                .padding(NavigationDrawerItemDefaults.ItemPadding)
                .padding(start = indent)
                .semantics { contentDescription = description },
    )
}

/** The name to show: the translated name for a standard folder, otherwise the server's decoded name. */
@Composable
fun folderName(folder: FolderSummary): String = folder.role?.let { stringResource(roleName(it)) } ?: folder.displayName

@StringRes
internal fun roleName(role: FolderRole): Int =
    when (role) {
        FolderRole.INBOX -> R.string.folder_inbox
        FolderRole.DRAFTS -> R.string.folder_drafts
        FolderRole.SENT -> R.string.folder_sent
        FolderRole.ARCHIVE -> R.string.folder_archive
        FolderRole.SPAM -> R.string.folder_spam
        FolderRole.TRASH -> R.string.folder_trash
        FolderRole.ALL_MAIL -> R.string.folder_all_mail
    }

@DrawableRes
internal fun folderIcon(role: FolderRole?): Int =
    when (role) {
        FolderRole.INBOX -> R.drawable.ic_mc_inbox
        FolderRole.DRAFTS -> R.drawable.ic_mc_drafts
        FolderRole.SENT -> R.drawable.ic_mc_sent
        FolderRole.ARCHIVE -> R.drawable.ic_mc_archive
        FolderRole.SPAM -> R.drawable.ic_mc_spam
        FolderRole.TRASH -> R.drawable.ic_mc_trash
        FolderRole.ALL_MAIL -> R.drawable.ic_mc_mail
        null -> R.drawable.ic_mc_folder
    }

@Composable
private fun unreadBadge(count: UInt): String =
    if (count > MAX_BADGE_COUNT) {
        stringResource(R.string.folder_unread_overflow, MAX_BADGE_COUNT.toInt())
    } else {
        stringResource(R.string.folder_unread_count, count.toInt())
    }
