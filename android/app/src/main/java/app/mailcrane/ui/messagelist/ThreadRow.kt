package app.mailcrane.ui.messagelist

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.ThreadSummary
import java.time.ZonedDateTime
import java.util.Locale

/** The row a conversation shows as: like a message, naming its people, with the message count (M4-02). */
internal fun threadAsSummary(
    thread: ThreadSummary,
    newest: MessageSummary?,
): MessageSummary =
    MessageSummary(
        id = thread.newestMessageId,
        fromName = thread.participants.joinToString(", "),
        fromAddr = newest?.fromAddr.orEmpty(),
        subject = thread.subject,
        snippet = newest?.snippet.orEmpty(),
        date = thread.lastDate,
        isSeen = thread.unreadCount == 0u,
        isFlagged = thread.isFlagged,
        hasAttachments = thread.hasAttachments,
    )

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun ThreadRow(
    thread: ThreadSummary,
    newest: MessageSummary?,
    now: ZonedDateTime,
    locale: Locale,
    onLongClick: (() -> Unit)? = null,
    selecting: Boolean = false,
    selected: Boolean = false,
    onClick: () -> Unit,
) {
    val count = thread.messageCount.toInt()
    val countLabel = pluralStringResource(R.plurals.thread_a11y_count, count, count)
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .combinedClickable(
                    onClick = onClick,
                    onLongClick = onLongClick,
                    onLongClickLabel = if (onLongClick != null) stringResource(R.string.message_select) else null,
                ).testTag("thread-row"),
    ) {
        MessageRow(
            message = threadAsSummary(thread, newest),
            now = now,
            locale = locale,
            modifier = Modifier.weight(1f),
            selecting = selecting,
            selected = selected,
        )
        Text(
            text = count.toString(),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(end = 16.dp).semantics { contentDescription = countLabel },
        )
    }
}
