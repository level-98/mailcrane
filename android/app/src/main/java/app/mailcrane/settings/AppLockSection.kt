package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.security.APP_LOCK_TIMEOUT_CHOICES
import app.mailcrane.security.AppLock
import app.mailcrane.security.DEFAULT_APP_LOCK_TIMEOUT_SECONDS
import app.mailcrane.security.DeviceAuth
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class AppLockSettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
        val appLock: AppLock,
    ) : ViewModel() {
        val enabled: StateFlow<Boolean> =
            settings.appLockEnabled.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)
        val timeoutSeconds: StateFlow<Int> =
            settings.appLockTimeoutSeconds.stateIn(
                viewModelScope,
                SharingStarted.WhileSubscribed(5_000),
                DEFAULT_APP_LOCK_TIMEOUT_SECONDS,
            )
        val allowScreenshots: StateFlow<Boolean> =
            settings.appLockAllowScreenshots.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

        fun setEnabled(enabled: Boolean) {
            viewModelScope.launch { settings.setAppLockEnabled(enabled) }
        }

        fun setTimeoutSeconds(seconds: Int) {
            viewModelScope.launch { settings.setAppLockTimeoutSeconds(seconds) }
        }

        fun setAllowScreenshots(allow: Boolean) {
            viewModelScope.launch { settings.setAppLockAllowScreenshots(allow) }
        }
    }

/**
 * "App lock" (M5-03). Turning it on or off asks for the device unlock first; without a screen lock
 * on the phone it can't be turned on, and says why.
 */
@Composable
fun AppLockSection(viewModel: AppLockSettingsViewModel = hiltViewModel()) {
    val enabled by viewModel.enabled.collectAsStateWithLifecycle()
    val timeout by viewModel.timeoutSeconds.collectAsStateWithLifecycle()
    val screenshots by viewModel.allowScreenshots.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val activity = context as? FragmentActivity
    val title = stringResource(R.string.app_lock_confirm_title)
    var noCredential by remember { mutableStateOf(false) }

    fun toggle(wanted: Boolean) {
        if (activity == null) return
        if (!DeviceAuth.available(context)) {
            noCredential = true
            return
        }
        noCredential = false
        viewModel.appLock.beginAuthentication()
        DeviceAuth.prompt(activity, title) { ok ->
            viewModel.appLock.endAuthentication(ok)
            if (ok) viewModel.setEnabled(wanted)
        }
    }

    Column(Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.app_lock_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        ListItem(
            supportingContent = {
                Text(
                    stringResource(
                        if (noCredential) R.string.app_lock_no_credential else R.string.app_lock_summary,
                    ),
                    color =
                        if (noCredential) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                )
            },
            trailingContent = { Switch(checked = enabled, onCheckedChange = null) },
            modifier = Modifier.toggleable(value = enabled, role = Role.Switch, onValueChange = { toggle(it) }),
        ) {
            Text(stringResource(R.string.app_lock_title))
        }
        if (enabled) {
            Column(Modifier.fillMaxWidth().selectableGroup()) {
                Text(
                    text = stringResource(R.string.app_lock_after),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp),
                )
                for (choice in APP_LOCK_TIMEOUT_CHOICES) {
                    ListItem(
                        leadingContent = { RadioButton(selected = timeout == choice, onClick = null) },
                        modifier =
                            Modifier.selectable(
                                selected = timeout == choice,
                                role = Role.RadioButton,
                                onClick = { viewModel.setTimeoutSeconds(choice) },
                            ),
                    ) { Text(lockAfterLabel(choice)) }
                }
            }
            ListItem(
                supportingContent = { Text(stringResource(R.string.app_lock_screenshots_summary)) },
                trailingContent = { Switch(checked = screenshots, onCheckedChange = null) },
                modifier =
                    Modifier.toggleable(
                        value = screenshots,
                        role = Role.Switch,
                        onValueChange = viewModel::setAllowScreenshots,
                    ),
            ) {
                Text(stringResource(R.string.app_lock_screenshots))
            }
        }
    }
}

@Composable
private fun lockAfterLabel(seconds: Int): String =
    when {
        seconds == 0 -> stringResource(R.string.app_lock_after_immediately)
        seconds < 3600 -> pluralStringResource(R.plurals.app_lock_after_minutes, seconds / 60, seconds / 60)
        else -> pluralStringResource(R.plurals.app_lock_after_hours, seconds / 3600, seconds / 3600)
    }
