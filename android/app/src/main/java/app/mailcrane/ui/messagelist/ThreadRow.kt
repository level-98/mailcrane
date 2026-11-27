package app.mailcrane.ui.messagelist

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
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
    onAvatarClick: (() -> Unit)? = null,
    onClick: () -> Unit,
) {
    MessageRow(
        message = threadAsSummary(thread, newest),
        now = now,
        locale = locale,
        modifier =
            Modifier
                .combinedClickable(
                    onClick = onClick,
                    onLongClick = onLongClick,
                    onLongClickLabel = if (onLongClick != null) stringResource(R.string.message_select) else null,
                ).testTag("thread-row"),
        selecting = selecting,
        selected = selected,
        onAvatarClick = onAvatarClick,
        // The picture shows the newest sender, not every participant.
        avatarName = newest?.fromName ?: thread.participants.firstOrNull().orEmpty(),
        // Under the date, inside the row, so it takes no width from the first line (#392).
        count = thread.messageCount.toInt(),
    )
}
