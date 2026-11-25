package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
    ) : ViewModel() {
        val cleanLinks: StateFlow<Boolean> =
            settings.cleanLinks.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)

        fun setCleanLinks(enabled: Boolean) {
            viewModelScope.launch { settings.setCleanLinks(enabled) }
        }
    }

/** The app's settings. */
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    accountId: Long? = null,
    viewModel: SettingsViewModel = hiltViewModel(),
) {
    val cleanLinks by viewModel.cleanLinks.collectAsStateWithLifecycle()
    SettingsContent(cleanLinks = cleanLinks, onCleanLinksChange = viewModel::setCleanLinks, onBack = onBack) {
        if (accountId != null) SignatureSection(accountId)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsContent(
    cleanLinks: Boolean,
    onCleanLinksChange: (Boolean) -> Unit,
    onBack: () -> Unit,
    accountSection: @Composable () -> Unit = {},
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_title)) },
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
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())) {
            // The whole row is one switch for TalkBack and touch; the Switch itself stays silent.
            ListItem(
                supportingContent = { Text(stringResource(R.string.settings_clean_links_summary)) },
                trailingContent = { Switch(checked = cleanLinks, onCheckedChange = null) },
                modifier =
                    Modifier.toggleable(value = cleanLinks, role = Role.Switch, onValueChange = onCleanLinksChange),
            ) {
                Text(stringResource(R.string.settings_clean_links))
            }
            accountSection()
        }
    }
}
