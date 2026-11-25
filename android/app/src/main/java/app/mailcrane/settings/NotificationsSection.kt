package app.mailcrane.settings

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
import app.mailcrane.notify.MailNotifier
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class NotificationSettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
        private val notifier: MailNotifier,
    ) : ViewModel() {
        val enabled: StateFlow<Boolean> =
            settings.notificationsEnabled.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)
        val alwaysOn: StateFlow<Boolean> =
            settings.alwaysOnSync.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

        /** Whether Android lets the app show notifications right now. */
        fun allowed(): Boolean = notifier.allowed()

        fun setEnabled(enabled: Boolean) {
            viewModelScope.launch { settings.setNotificationsEnabled(enabled) }
        }

        fun setAlwaysOn(enabled: Boolean) {
            viewModelScope.launch { settings.setAlwaysOnSync(enabled) }
        }
    }

/** "Notifications" (M4-05): new mail notifications, and always-on sync. */
@Composable
fun NotificationsSection(viewModel: NotificationSettingsViewModel = hiltViewModel()) {
    val enabled by viewModel.enabled.collectAsStateWithLifecycle()
    val alwaysOn by viewModel.alwaysOn.collectAsStateWithLifecycle()
    var allowed by remember { mutableStateOf(viewModel.allowed()) }
    val askPermission =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            allowed = viewModel.allowed()
            if (granted) viewModel.setEnabled(true)
        }

    fun toggle(wanted: Boolean) {
        if (wanted && !viewModel.allowed() && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            askPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            return
        }
        viewModel.setEnabled(wanted)
    }

    val on = enabled && allowed
    val blocked = enabled && !allowed
    Column(Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.settings_notifications_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        ListItem(
            supportingContent = {
                Text(
                    stringResource(
                        if (blocked) {
                            R.string.settings_notifications_blocked
                        } else {
                            R.string.settings_notifications_summary
                        },
                    ),
                    color =
                        if (blocked) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                )
            },
            trailingContent = { Switch(checked = on, onCheckedChange = null) },
            modifier = Modifier.toggleable(value = on, role = Role.Switch, onValueChange = { toggle(it) }),
        ) {
            Text(stringResource(R.string.settings_notifications))
        }
        if (enabled) {
            ListItem(
                supportingContent = { Text(stringResource(R.string.settings_always_on_summary)) },
                trailingContent = { Switch(checked = alwaysOn, onCheckedChange = null) },
                modifier =
                    Modifier.toggleable(value = alwaysOn, role = Role.Switch, onValueChange = viewModel::setAlwaysOn),
            ) {
                Text(stringResource(R.string.settings_always_on))
            }
        }
    }
}
