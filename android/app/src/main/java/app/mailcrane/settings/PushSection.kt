package app.mailcrane.settings

import android.app.Activity
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.push.FailureKind
import app.mailcrane.push.PushStatus
import app.mailcrane.push.UnifiedPushControl
import app.mailcrane.push.UnifiedPushState

/**
 * "Push wake-ups (UnifiedPush)" (M4-05), under Notifications: wakes Mailcrane through the user's
 * UnifiedPush distributor instead of a permanent connection. Off by default; the address it gets
 * has to be given to a gateway that watches the mailbox (IMAP servers don't push by themselves).
 */
@Composable
fun PushSection() {
    val context = LocalContext.current
    val state = remember { UnifiedPushState.get(context) }
    val status by state.status.collectAsStateWithLifecycle()
    var hasDistributor by remember { mutableStateOf(UnifiedPushControl.hasDistributor(context)) }
    // A distributor may have been installed while the app was in the background.
    LifecycleResumeEffect(Unit) {
        hasDistributor = UnifiedPushControl.hasDistributor(context)
        onPauseOrDispose {}
    }
    PushSettings(
        status = status,
        hasDistributor = hasDistributor,
        onToggle = { on ->
            val activity = context.findActivity()
            when {
                !on -> UnifiedPushControl.turnOff(context)
                activity != null -> UnifiedPushControl.turnOn(activity)
            }
        },
        onCopy = { copyEndpoint(context, it) },
    )
}

/** The push wake-up rows for [status]; stateless, so tests can show every state. */
@Composable
fun PushSettings(
    status: PushStatus,
    hasDistributor: Boolean,
    onToggle: (Boolean) -> Unit,
    onCopy: (String) -> Unit,
) {
    val on = status != PushStatus.Off
    // Turning it off always works; turning it on needs a distributor.
    val canToggle = on || hasDistributor
    Column(Modifier.fillMaxWidth()) {
        ListItem(
            supportingContent = {
                Text(
                    stringResource(
                        if (hasDistributor || on) R.string.push_summary else R.string.push_no_distributor,
                    ),
                )
            },
            trailingContent = { Switch(checked = on, onCheckedChange = null, enabled = canToggle) },
            modifier =
                Modifier.toggleable(
                    value = on,
                    enabled = canToggle,
                    role = Role.Switch,
                    onValueChange = onToggle,
                ),
        ) {
            Text(stringResource(R.string.push_title))
        }
        when (status) {
            PushStatus.Off -> Unit
            PushStatus.Registering -> StatusText(stringResource(R.string.push_registering))
            PushStatus.NoDistributor -> StatusText(stringResource(R.string.push_no_distributor), error = true)
            is PushStatus.Failed -> StatusText(stringResource(failureText(status.reason)), error = true)
            is PushStatus.Ready -> {
                StatusText(stringResource(R.string.push_ready))
                Text(
                    status.endpoint,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(horizontal = 16.dp),
                )
                if (status.temporary) StatusText(stringResource(R.string.push_temporary))
                TextButton(onClick = { onCopy(status.endpoint) }, modifier = Modifier.padding(horizontal = 8.dp)) {
                    Text(stringResource(R.string.push_copy))
                }
            }
        }
    }
}

@Composable
private fun StatusText(
    text: String,
    error: Boolean = false,
) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = if (error) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp),
    )
}

private fun failureText(reason: FailureKind): Int =
    when (reason) {
        FailureKind.NETWORK -> R.string.push_failed_network
        FailureKind.ACTION_REQUIRED -> R.string.push_failed_action
        FailureKind.NEEDS_SERVER_KEY -> R.string.push_failed_vapid
        FailureKind.OTHER -> R.string.push_failed_other
        FailureKind.UNREGISTERED -> R.string.push_failed_unregistered
    }

/** Copies the push address, marked sensitive so Android 13+ doesn't preview it. */
private fun copyEndpoint(
    context: Context,
    endpoint: String,
) {
    val clip = ClipData.newPlainText(context.getString(R.string.push_title), endpoint)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
    }
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(clip)
}

private tailrec fun Context.findActivity(): Activity? =
    when (this) {
        is Activity -> this
        is ContextWrapper -> baseContext.findActivity()
        else -> null
    }
