package app.mailcrane.settings

import androidx.activity.compose.LocalActivity
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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
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
import androidx.lifecycle.HasDefaultViewModelProviderFactory
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.viewmodel.CreationExtras
import androidx.lifecycle.viewmodel.compose.LocalViewModelStoreOwner
import androidx.lifecycle.viewmodel.compose.viewModel
import app.mailcrane.R
import app.mailcrane.core.AccountSummary
import java.util.UUID

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
            // Its own view models, new each time the page opens (#500): an account added after one was
            // removed can get the same id, and must never see that account's signature, colour or
            // authentication server name (which decides what "sender verified" trusts).
            PageViewModels(account.id) { sections(account.id) }
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

/**
 * Gives [content] a view model store of its own for one opening of an account's page (#500). It lives
 * while the page is open, rotation included (a saved token names it), and is cleared when the page
 * closes, by Back or after Remove, so a later page for a reused account id starts from that account's
 * own saved settings. Hilt's factory comes from the activity, so `hiltViewModel` works as before.
 */
@Composable
private fun PageViewModels(
    accountId: Long,
    content: @Composable () -> Unit,
) {
    val parent = LocalViewModelStoreOwner.current
    if (parent == null) {
        content()
        return
    }
    val stores: AccountPageStores = viewModel(viewModelStoreOwner = parent)
    val opening = rememberSaveable(accountId) { UUID.randomUUID().toString() }
    val key = "account-page-$accountId-$opening"
    val owner = remember(key) { PageViewModelStoreOwner(stores.store(key), parent) }
    val activity = LocalActivity.current
    DisposableEffect(key) {
        // Kept across a rotation; gone once the page is really closed.
        onDispose { if (activity?.isChangingConfigurations != true) stores.close(key) }
    }
    CompositionLocalProvider(LocalViewModelStoreOwner provides owner) { content() }
}

/** The view model stores of the account pages that are open, kept by the activity across rotation. */
internal class AccountPageStores : ViewModel() {
    private val stores = mutableMapOf<String, ViewModelStore>()

    fun store(key: String): ViewModelStore = stores.getOrPut(key) { ViewModelStore() }

    /** Clears the page's view models and forgets its store. */
    fun close(key: String) {
        stores.remove(key)?.clear()
    }

    override fun onCleared() {
        stores.values.forEach { it.clear() }
        stores.clear()
    }
}

/**
 * One page's store, with the activity's factory and extras, so Hilt (and assisted) view models still work.
 * Known limit (#492 review): the extras still name the activity as the owner of saved state, so a
 * `SavedStateHandle` would be shared across openings under the same key. Nothing on these pages uses
 * one; if one is ever needed, move to lifecycle's `rememberViewModelStoreOwner()` first (#518).
 */
private class PageViewModelStoreOwner(
    override val viewModelStore: ViewModelStore,
    private val parent: ViewModelStoreOwner,
) : ViewModelStoreOwner,
    HasDefaultViewModelProviderFactory {
    override val defaultViewModelProviderFactory: ViewModelProvider.Factory
        get() =
            (parent as? HasDefaultViewModelProviderFactory)?.defaultViewModelProviderFactory
                ?: ViewModelProvider.NewInstanceFactory()

    override val defaultViewModelCreationExtras: CreationExtras
        get() = (parent as? HasDefaultViewModelProviderFactory)?.defaultViewModelCreationExtras ?: CreationExtras.Empty
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
