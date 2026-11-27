package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.AccountSummary

/**
 * One account's own settings page (#396), opened from its row in Settings › Accounts: titled with the
 * account's name and address, holding that account's options ([sections], by default signature, Sent
 * copy, colour and Advanced) and Remove. Every option edits [account] only, whichever mailbox was open
 * behind Settings.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AccountSettingsPage(
    account: AccountSummary,
    onBack: () -> Unit,
    onRemove: (Long) -> Unit,
    onChanged: () -> Unit = {},
    sections: @Composable (Long) -> Unit = { id -> AccountSections(id, onChanged) },
) {
    var confirmRemove by rememberSaveable { mutableStateOf(false) }
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column(Modifier.semantics(mergeDescendants = true) { heading() }) {
                        Text(accountName(account), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (showsAddressApart(account)) {
                            Text(
                                account.email,
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
                            painter = painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.settings_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .testTag("account-settings-${account.id}"),
        ) {
            sections(account.id)
            OutlinedButton(
                onClick = { confirmRemove = true },
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 24.dp),
            ) {
                Text(stringResource(R.string.accounts_remove_account))
            }
        }
    }
    if (confirmRemove) {
        AlertDialog(
            onDismissRequest = { confirmRemove = false },
            title = { Text(stringResource(R.string.accounts_remove_title, account.email)) },
            text = { Text(stringResource(R.string.accounts_remove_text)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = false
                    onRemove(account.id)
                    onBack()
                }) { Text(stringResource(R.string.accounts_remove)) }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = false }) { Text(stringResource(R.string.accounts_cancel)) }
            },
        )
    }
}

/** The options that belong to one account, each editing that account only. */
@Composable
private fun AccountSections(
    accountId: Long,
    onChanged: () -> Unit,
) {
    SignatureSection(accountId)
    SaveSentSection(accountId)
    AccountColorSection(accountId, onChanged = onChanged)
    AccountAdvancedSection(accountId)
}

/** An account's name, or its address when it has none. */
internal fun accountName(account: AccountSummary): String = account.displayName.ifBlank { account.email }

/** Whether the address is worth showing under the name: not when the name is the address (#396). */
internal fun showsAddressApart(account: AccountSummary): Boolean =
    account.displayName.isNotBlank() && !account.displayName.trim().equals(account.email.trim(), ignoreCase = true)
