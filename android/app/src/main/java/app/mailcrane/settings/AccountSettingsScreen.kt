package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.Button
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.AccountSettingsCore
import app.mailcrane.core.AccountSignature
import app.mailcrane.core.SaveSentSetting
import app.mailcrane.core.SettingsFailure
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** How the last save of the signature went. */
enum class SaveStatus { NONE, SAVED, TOO_LONG, FAILED }

data class SignatureState(
    val loaded: Boolean = false,
    val text: String = "",
    val enabled: Boolean = true,
    /** What is saved, to tell whether there are unsaved changes. */
    val saved: AccountSignature? = null,
    val status: SaveStatus = SaveStatus.NONE,
    /** "Save sent messages to the Sent folder"; null until loaded. */
    val saveSent: SaveSentSetting? = null,
) {
    val dirty: Boolean get() = loaded && saved != AccountSignature(text, enabled)
}

/** The signature setting of one account. */
@HiltViewModel(assistedFactory = AccountSettingsViewModel.Factory::class)
class AccountSettingsViewModel
    @AssistedInject
    constructor(
        private val core: AccountSettingsCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): AccountSettingsViewModel
        }

        private val _state = MutableStateFlow(SignatureState())
        val state: StateFlow<SignatureState> = _state.asStateFlow()

        init {
            viewModelScope.launch {
                try {
                    val saved = core.getSignature(accountId)
                    val saveSent = core.getSaveSent(accountId)
                    _state.update {
                        SignatureState(
                            loaded = true,
                            text = saved.text,
                            enabled = saved.enabled,
                            saved = saved,
                            saveSent = saveSent,
                        )
                    }
                } catch (_: SettingsFailure) {
                    // Nothing to edit; the section stays hidden.
                }
            }
        }

        /** Saved at once; there's nothing to type. */
        fun onSaveSentChange(setting: SaveSentSetting) {
            val before = _state.value.saveSent
            _state.update { it.copy(saveSent = setting) }
            viewModelScope.launch {
                try {
                    core.setSaveSent(accountId, setting)
                } catch (_: SettingsFailure) {
                    _state.update { it.copy(saveSent = before) }
                }
            }
        }

        fun onTextChange(text: String) = _state.update { it.copy(text = text, status = SaveStatus.NONE) }

        fun onEnabledChange(enabled: Boolean) = _state.update { it.copy(enabled = enabled, status = SaveStatus.NONE) }

        fun save() {
            val current = _state.value
            if (!current.dirty) return
            val signature = AccountSignature(current.text, current.enabled)
            viewModelScope.launch {
                val status =
                    try {
                        core.setSignature(accountId, signature)
                        SaveStatus.SAVED
                    } catch (_: SettingsFailure.TooLong) {
                        SaveStatus.TOO_LONG
                    } catch (_: SettingsFailure) {
                        SaveStatus.FAILED
                    }
                _state.update {
                    if (status == SaveStatus.SAVED) {
                        it.copy(saved = signature, status = status)
                    } else {
                        it.copy(status = status)
                    }
                }
            }
        }
    }

/** The "Save sent messages" choice of the settings screen, for account [accountId]. */
@Composable
fun SaveSentSection(
    accountId: Long,
    viewModel: AccountSettingsViewModel =
        hiltViewModel<AccountSettingsViewModel, AccountSettingsViewModel.Factory>(
            key = "account-settings-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val current = state.saveSent ?: return
    SaveSentContent(current, viewModel::onSaveSentChange)
}

@Composable
fun SaveSentContent(
    current: SaveSentSetting,
    onChange: (SaveSentSetting) -> Unit,
) {
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(R.string.settings_save_sent_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for ((setting, label, summary) in SAVE_SENT_OPTIONS) {
            ListItem(
                leadingContent = { RadioButton(selected = current == setting, onClick = null) },
                supportingContent = summary?.let { { Text(stringResource(it)) } },
                modifier =
                    Modifier.selectable(
                        selected = current == setting,
                        role = Role.RadioButton,
                        onClick = { onChange(setting) },
                    ),
            ) {
                Text(stringResource(label))
            }
        }
    }
}

private val SAVE_SENT_OPTIONS =
    listOf(
        Triple(SaveSentSetting.AUTOMATIC, R.string.settings_save_sent_auto, R.string.settings_save_sent_auto_summary),
        Triple(SaveSentSetting.ON, R.string.settings_save_sent_on, null),
        Triple(SaveSentSetting.OFF, R.string.settings_save_sent_off, null),
    )

/** The signature section of the settings screen, for account [accountId]. */
@Composable
fun SignatureSection(
    accountId: Long,
    viewModel: AccountSettingsViewModel =
        hiltViewModel<AccountSettingsViewModel, AccountSettingsViewModel.Factory>(
            key = "account-settings-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    if (state.loaded) {
        SignatureContent(
            state = state,
            onTextChange = viewModel::onTextChange,
            onEnabledChange = viewModel::onEnabledChange,
            onSave = viewModel::save,
        )
    }
}

@Composable
fun SignatureContent(
    state: SignatureState,
    onTextChange: (String) -> Unit,
    onEnabledChange: (Boolean) -> Unit,
    onSave: () -> Unit,
) {
    Column(Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.settings_signature_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        ListItem(
            trailingContent = { Switch(checked = state.enabled, onCheckedChange = null) },
            modifier = Modifier.toggleable(value = state.enabled, role = Role.Switch, onValueChange = onEnabledChange),
        ) {
            Text(stringResource(R.string.settings_signature_enabled))
        }
        OutlinedTextField(
            value = state.text,
            onValueChange = onTextChange,
            label = { Text(stringResource(R.string.settings_signature_field)) },
            minLines = 3,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        )
        Button(
            onClick = onSave,
            enabled = state.dirty,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        ) {
            Text(stringResource(R.string.settings_signature_save))
        }
        val status =
            when (state.status) {
                SaveStatus.NONE -> null
                SaveStatus.SAVED -> R.string.settings_signature_saved
                SaveStatus.TOO_LONG -> R.string.settings_signature_too_long
                SaveStatus.FAILED -> R.string.settings_signature_failed
            }
        if (status != null) {
            Text(
                text = stringResource(status),
                color =
                    if (state.status == SaveStatus.SAVED) {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    } else {
                        MaterialTheme.colorScheme.error
                    },
                modifier = Modifier.padding(horizontal = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
    }
}
