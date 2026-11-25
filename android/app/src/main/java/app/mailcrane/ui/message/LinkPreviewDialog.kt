package app.mailcrane.ui.message

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.widget.Toast
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.LinkWarning

/**
 * Shows where a link goes before it opens (M5-07). Everything shown, the host and each warning,
 * comes from the core's `check_link`; this only draws it. A hint, never a block: Open is always
 * there, except for links that aren't web links.
 */
@Composable
fun LinkPreviewDialog(
    preview: LinkPreview,
    onOpen: () -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val host = preview.check.displayHost
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.link_preview_title)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(stringResource(R.string.link_preview_host_label), style = MaterialTheme.typography.labelMedium)
                Text(host.ifEmpty { preview.url }, style = MaterialTheme.typography.headlineSmall)
                Text(preview.url, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp))
                for (warning in preview.check.warnings) {
                    Text(
                        warningText(warning, host),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                }
            }
        },
        confirmButton = {
            if (LinkWarning.UnsupportedScheme !in preview.check.warnings) {
                TextButton(onClick = onOpen) { Text(stringResource(R.string.link_preview_open)) }
            }
        },
        dismissButton = {
            Column {
                TextButton(onClick = {
                    copyLink(context, preview.url)
                    onDismiss()
                }) { Text(stringResource(R.string.link_preview_copy)) }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.link_preview_cancel)) }
            }
        },
    )
}

@Composable
private fun warningText(
    warning: LinkWarning,
    host: String,
): String =
    when (warning) {
        LinkWarning.UnsupportedScheme -> stringResource(R.string.link_warning_unsupported)
        LinkWarning.MixedScript -> stringResource(R.string.link_warning_mixed_script)
        is LinkWarning.TextHostMismatch -> stringResource(R.string.link_warning_mismatch, warning.textHost, host)
        LinkWarning.IpAddress -> stringResource(R.string.link_warning_ip)
        LinkWarning.UserInfo -> stringResource(R.string.link_warning_userinfo, host)
        LinkWarning.Http -> stringResource(R.string.link_warning_http)
        LinkWarning.TooLong -> stringResource(R.string.link_warning_too_long)
    }

private fun copyLink(
    context: Context,
    url: String,
) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText(null, url))
    // Android 13 and later confirm a copy themselves.
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
        Toast.makeText(context, R.string.link_preview_copied, Toast.LENGTH_SHORT).show()
    }
}
