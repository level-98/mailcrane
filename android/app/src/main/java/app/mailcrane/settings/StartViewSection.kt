package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.AccountSummary
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

/** Settings → "Open the app on" (M4-16). */
@HiltViewModel
class StartViewSettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
    ) : ViewModel() {
        val startView: StateFlow<StartView> =
            settings.startView.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), StartView.Last)

        fun setStartView(view: StartView) {
            viewModelScope.launch { settings.setStartView(view) }
        }
    }

@Composable
fun StartViewSection(
    accounts: List<AccountSummary>,
    viewModel: StartViewSettingsViewModel = hiltViewModel(),
) {
    val current by viewModel.startView.collectAsStateWithLifecycle()
    StartViewContent(accounts, current, viewModel::setStartView)
}

/** The choices offered: the Unified Inbox (with two or more accounts), each account's inbox, and Last used. */
internal fun startViewChoices(accounts: List<AccountSummary>): List<StartView> =
    buildList {
        if (accounts.size > 1) add(StartView.Unified)
        accounts.forEach { add(StartView.Account(it.id)) }
        add(StartView.Last)
    }

@Composable
internal fun StartViewContent(
    accounts: List<AccountSummary>,
    current: StartView,
    onChange: (StartView) -> Unit,
) {
    // A setting for an account that no longer exists reads as Last used (the app falls back to it).
    val shown =
        if (current is StartView.Account && accounts.none { it.id == current.accountId }) StartView.Last else current
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(R.string.settings_start_view),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for (choice in startViewChoices(accounts)) {
            ListItem(
                leadingContent = { RadioButton(selected = shown == choice, onClick = null) },
                modifier =
                    Modifier.selectable(
                        selected = shown == choice,
                        role = Role.RadioButton,
                        onClick = { onChange(choice) },
                    ),
            ) {
                Text(
                    when (choice) {
                        StartView.Unified -> stringResource(R.string.all_inboxes)
                        StartView.Last -> stringResource(R.string.start_view_last)
                        is StartView.Account ->
                            stringResource(
                                R.string.start_view_account,
                                accounts.firstOrNull { it.id == choice.accountId }?.email.orEmpty(),
                            )
                    },
                )
            }
        }
    }
}
