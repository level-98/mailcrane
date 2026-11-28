package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.AccountSettingsCore
import app.mailcrane.core.SenderTrustChanges
import app.mailcrane.core.SettingsFailure
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** How the last save of the server name went. */
enum class AuthservSaveStatus { NONE, SAVED, CLEARED, INVALID, FAILED }

data class ProviderAuthservState(
    val loaded: Boolean = false,
    val text: String = "",
    /** What is saved (null when unset), to tell whether there are unsaved changes. */
    val saved: String? = null,
    val status: AuthservSaveStatus = AuthservSaveStatus.NONE,
) {
    val dirty: Boolean get() = loaded && text.trim() != saved.orEmpty()
}

/** The account's "My provider's authentication server name" (M5-08). The core cleans and checks it. */
@HiltViewModel(assistedFactory = ProviderAuthservViewModel.Factory::class)
class ProviderAuthservViewModel
    @AssistedInject
    constructor(
        private val core: AccountSettingsCore,
        private val senderTrust: SenderTrustChanges,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): ProviderAuthservViewModel
        }

        private val _state = MutableStateFlow(ProviderAuthservState())
        val state: StateFlow<ProviderAuthservState> = _state.asStateFlow()

        init {
            viewModelScope.launch {
                try {
                    val saved = core.getProviderAuthservId(accountId)
                    _state.update { ProviderAuthservState(loaded = true, text = saved.orEmpty(), saved = saved) }
                } catch (_: SettingsFailure) {
                    // Nothing to edit; the section stays hidden.
                }
            }
        }

        fun onTextChange(text: String) = _state.update { it.copy(text = text, status = AuthservSaveStatus.NONE) }

        fun save() {
            val current = _state.value
            if (!current.dirty) return
            viewModelScope.launch {
                try {
                    // The write and the trust refresh are one step: closing the page clears this view model
                    // (#500), and a save cut off between them would leave open messages on the old trust
                    // (#492 review). Open messages must show the new trust at once, lowered trust above all.
                    val saved =
                        withContext(NonCancellable) {
                            core.setProviderAuthservId(accountId, current.text).also { senderTrust.changed() }
                        }
                    _state.update {
                        it.copy(
                            text = saved.orEmpty(),
                            saved = saved,
                            status = if (saved == null) AuthservSaveStatus.CLEARED else AuthservSaveStatus.SAVED,
                        )
                    }
                } catch (_: SettingsFailure.InvalidAuthservId) {
                    _state.update { it.copy(status = AuthservSaveStatus.INVALID) }
                } catch (_: SettingsFailure) {
                    _state.update { it.copy(status = AuthservSaveStatus.FAILED) }
                }
            }
        }
    }

/** The per-account "Advanced" section of the settings screen, for account [accountId]. */
@Composable
fun AccountAdvancedSection(
    accountId: Long,
    viewModel: ProviderAuthservViewModel =
        hiltViewModel<ProviderAuthservViewModel, ProviderAuthservViewModel.Factory>(
            key = "provider-authserv-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    if (state.loaded) {
        AccountAdvancedContent(state = state, onTextChange = viewModel::onTextChange, onSave = viewModel::save)
    }
}

@Composable
fun AccountAdvancedContent(
    state: ProviderAuthservState,
    onTextChange: (String) -> Unit,
    onSave: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().testTag("account-advanced")) {
        Text(
            text = stringResource(R.string.settings_account_advanced_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        OutlinedTextField(
            value = state.text,
            onValueChange = onTextChange,
            label = { Text(stringResource(R.string.settings_authserv_field)) },
            placeholder = { Text(stringResource(R.string.settings_authserv_placeholder)) },
            supportingText = { Text(stringResource(R.string.settings_authserv_explain)) },
            isError = state.status == AuthservSaveStatus.INVALID,
            singleLine = true,
            keyboardOptions =
                KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Uri,
                    imeAction = ImeAction.Done,
                ),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("authserv-field"),
        )
        Button(
            onClick = onSave,
            enabled = state.dirty,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        ) {
            Text(stringResource(R.string.settings_authserv_save))
        }
        val status =
            when (state.status) {
                AuthservSaveStatus.NONE -> null
                AuthservSaveStatus.SAVED -> R.string.settings_authserv_saved
                AuthservSaveStatus.CLEARED -> R.string.settings_authserv_cleared
                AuthservSaveStatus.INVALID -> R.string.settings_authserv_invalid
                AuthservSaveStatus.FAILED -> R.string.settings_authserv_failed
            }
        if (status != null) {
            val ok = state.status == AuthservSaveStatus.SAVED || state.status == AuthservSaveStatus.CLEARED
            Text(
                text = stringResource(status),
                color = if (ok) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
    }
}
