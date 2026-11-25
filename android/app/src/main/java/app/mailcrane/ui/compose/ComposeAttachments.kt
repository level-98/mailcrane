package app.mailcrane.ui.compose

import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.InputChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.mailcrane.R

/** Attached files as chips (name, size, remove), with the size warning (M2-04). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun ComposeAttachments(
    state: ComposeUiState,
    onRemove: (Int) -> Unit,
) {
    if (state.attachments.isEmpty() && state.forwardAttachmentsOf == null) return
    val context = LocalContext.current
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("compose-attachments")) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            state.attachments.forEachIndexed { index, a ->
                val size = Formatter.formatShortFileSize(context, a.bytes.size.toLong())
                val remove = stringResource(R.string.compose_remove_attachment, a.filename)
                InputChip(
                    selected = false,
                    onClick = { onRemove(index) },
                    enabled = !state.sending,
                    label = { Text("${a.filename} · $size", maxLines = 1) },
                    trailingIcon = {
                        Icon(
                            painterResource(R.drawable.ic_mc_close),
                            contentDescription = remove,
                            modifier = Modifier.size(18.dp),
                        )
                    },
                )
            }
        }
        if (state.forwardAttachmentsOf != null) {
            Text(
                text = stringResource(R.string.compose_forward_attachments),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (state.attachmentsTooLarge) {
            Text(
                text = stringResource(R.string.compose_attachments_large),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}
