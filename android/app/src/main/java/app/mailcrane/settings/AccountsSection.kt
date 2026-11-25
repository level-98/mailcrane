package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.AccountSummary
import app.mailcrane.ui.components.AccountDot

/** The saved accounts, with Add and Remove (M4-01). Removing asks first. */
@Composable
internal fun AccountsSection(
    accounts: List<AccountSummary>,
    onAdd: () -> Unit,
    onRemove: (Long) -> Unit,
) {
    var removing by rememberSaveable { mutableStateOf<Long?>(null) }
    Column(Modifier.fillMaxWidth().testTag("accounts-section")) {
        Text(
            text = stringResource(R.string.accounts_title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        accounts.forEach { account ->
            ListItem(
                leadingContent = { AccountDot(account.color.toInt()) },
                headlineContent = { Text(account.displayName.ifBlank { account.email }) },
                supportingContent = { Text(account.email) },
                trailingContent = {
                    TextButton(onClick = { removing = account.id }) { Text(stringResource(R.string.accounts_remove)) }
                },
            )
        }
        TextButton(onClick = onAdd, modifier = Modifier.padding(horizontal = 8.dp)) {
            Text(stringResource(R.string.accounts_add))
        }
    }
    val target = accounts.firstOrNull { it.id == removing }
    if (target != null) {
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text(stringResource(R.string.accounts_remove_title, target.email)) },
            text = { Text(stringResource(R.string.accounts_remove_text)) },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    onRemove(target.id)
                }) { Text(stringResource(R.string.accounts_remove)) }
            },
            dismissButton = {
                TextButton(onClick = { removing = null }) { Text(stringResource(R.string.accounts_cancel)) }
            },
        )
    }
}
