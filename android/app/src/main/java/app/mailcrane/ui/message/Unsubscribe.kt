package app.mailcrane.ui.message

import android.net.Uri
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.UnsubscribeInfo

/** The host of an unsubscribe page, for the confirmation ("example.net"). */
internal fun unsubscribeHost(url: String?): String = url?.let { Uri.parse(it).host }.orEmpty()

/**
 * "Unsubscribe" under the message header (M4-10). Tapping asks first, saying which method will be used;
 * nothing happens until the user confirms.
 */
@Composable
internal fun UnsubscribeAction(
    info: UnsubscribeInfo,
    busy: Boolean,
    onConfirm: () -> Unit,
) {
    val method = unsubscribeMethod(info) ?: return
    var asking by rememberSaveable { mutableStateOf(false) }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.padding(horizontal = 8.dp).testTag("unsubscribe"),
    ) {
        TextButton(onClick = { asking = true }, enabled = !busy) { Text(stringResource(R.string.unsubscribe_button)) }
        if (busy) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
    }
    if (asking) {
        UnsubscribeDialog(
            info = info,
            method = method,
            onConfirm = {
                asking = false
                onConfirm()
            },
            onDismiss = { asking = false },
        )
    }
}

@Composable
internal fun UnsubscribeDialog(
    info: UnsubscribeInfo,
    method: UnsubscribeMethod,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val sender = info.sender.ifBlank { stringResource(R.string.message_unknown_sender) }
    val explanation =
        when (method) {
            UnsubscribeMethod.ONE_CLICK ->
                stringResource(
                    R.string.unsubscribe_how_one_click,
                    unsubscribeHost(info.webUrl),
                )
            UnsubscribeMethod.EMAIL -> stringResource(R.string.unsubscribe_how_email, info.mailto?.address.orEmpty())
            UnsubscribeMethod.WEB -> stringResource(R.string.unsubscribe_how_web, unsubscribeHost(info.webUrl))
        }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.unsubscribe_confirm_title, sender)) },
        text = { Text(explanation) },
        confirmButton = { TextButton(onClick = onConfirm) { Text(stringResource(R.string.unsubscribe_button)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.unsubscribe_cancel)) } },
    )
}
