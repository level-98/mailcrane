package app.mailcrane.settings

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.notify.MailNotifier
import app.mailcrane.sync.BatteryOptimisation
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
    val context = LocalContext.current
    var exempt by remember { mutableStateOf(BatteryOptimisation.isExempt(context)) }
    var explaining by rememberSaveable { mutableStateOf(false) }
    // Coming back from the system's battery screen.
    LifecycleResumeEffect(Unit) {
        exempt = BatteryOptimisation.isExempt(context)
        allowed = viewModel.allowed()
        onPauseOrDispose {}
    }
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
                    Modifier.toggleable(value = alwaysOn, role = Role.Switch, onValueChange = { turnOn ->
                        viewModel.setAlwaysOn(turnOn)
                        // Explain battery optimisation right when it matters.
                        if (turnOn && !exempt) explaining = true
                    }),
            ) {
                Text(stringResource(R.string.settings_always_on))
            }
            if (alwaysOn) BatteryRow(exempt = exempt, onClick = { explaining = true })
        }
    }
    if (explaining) {
        BatteryExplanation(onDismiss = { explaining = false })
    }
}

/** Under always-on sync: whether battery optimisation could delay notifications. */
@Composable
private fun BatteryRow(
    exempt: Boolean,
    onClick: () -> Unit,
) {
    if (exempt) {
        ListItem(headlineContent = { Text(stringResource(R.string.settings_battery_exempt)) })
        return
    }
    ListItem(
        headlineContent = {
            Text(stringResource(R.string.settings_battery_warning), color = MaterialTheme.colorScheme.error)
        },
        supportingContent = { Text(stringResource(R.string.settings_battery_warning_summary)) },
        modifier = Modifier.clickable(role = Role.Button, onClick = onClick),
    )
}

/**
 * Why battery optimisation delays always-on notifications, and how to turn it off (M4-05). Opens
 * the system's own screen: the user makes the change there.
 */
@Composable
fun BatteryExplanation(onDismiss: () -> Unit) {
    val context = LocalContext.current
    var failed by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.battery_explain_title)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(stringResource(R.string.battery_explain_body))
                if (failed) {
                    Text(
                        stringResource(R.string.battery_explain_failed),
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                if (BatteryOptimisation.openSettings(context)) onDismiss() else failed = true
            }) { Text(stringResource(R.string.battery_explain_open)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.battery_explain_later)) }
        },
    )
}
