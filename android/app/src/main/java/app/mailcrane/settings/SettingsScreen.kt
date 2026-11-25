package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
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

        val undoSendSeconds: StateFlow<Int> =
            settings.undoSendSeconds.stateIn(
                viewModelScope,
                SharingStarted.WhileSubscribed(5_000),
                DEFAULT_UNDO_SEND_SECONDS,
            )

        fun setUndoSendSeconds(seconds: Int) {
            viewModelScope.launch { settings.setUndoSendSeconds(seconds) }
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
    val undoSend by viewModel.undoSendSeconds.collectAsStateWithLifecycle()
    SettingsContent(
        cleanLinks = cleanLinks,
        onCleanLinksChange = viewModel::setCleanLinks,
        onBack = onBack,
        undoSendSeconds = undoSend,
        onUndoSendChange = viewModel::setUndoSendSeconds,
    ) {
        if (accountId != null) {
            SignatureSection(accountId)
            SaveSentSection(accountId)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsContent(
    cleanLinks: Boolean,
    onCleanLinksChange: (Boolean) -> Unit,
    onBack: () -> Unit,
    undoSendSeconds: Int = DEFAULT_UNDO_SEND_SECONDS,
    onUndoSendChange: (Int) -> Unit = {},
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
            UndoSendSection(undoSendSeconds, onUndoSendChange)
            accountSection()
        }
    }
}

/** "Undo send": how long a sent message waits, so it can be taken back. */
@Composable
private fun UndoSendSection(
    seconds: Int,
    onChange: (Int) -> Unit,
) {
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(R.string.settings_undo_send_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for (choice in UNDO_SEND_CHOICES) {
            ListItem(
                leadingContent = { RadioButton(selected = seconds == choice, onClick = null) },
                modifier =
                    Modifier.selectable(
                        selected = seconds == choice,
                        role = Role.RadioButton,
                        onClick = { onChange(choice) },
                    ),
            ) {
                Text(
                    if (choice == 0) {
                        stringResource(R.string.settings_undo_send_off)
                    } else {
                        pluralStringResource(R.plurals.settings_undo_send_seconds, choice, choice)
                    },
                )
            }
        }
    }
}
