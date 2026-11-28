package app.mailcrane.ui.messagelist

import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.settings.SwipeAction

/**
 * The core action a swipe of [action] runs on [message], and whether it offers Undo; null for none and
 * for move, which asks for a folder first. Shared by the message list and a conversation (#552).
 */
internal fun swipeCoreAction(
    message: MessageSummary,
    action: SwipeAction,
): Pair<MessageAction, Boolean>? =
    when (action) {
        SwipeAction.NONE, SwipeAction.MOVE -> null
        SwipeAction.ARCHIVE -> MessageAction.Archive to true
        SwipeAction.DELETE -> MessageAction.Delete to true
        SwipeAction.READ_UNREAD -> (if (message.isSeen) MessageAction.MarkUnread else MessageAction.MarkRead) to false
        SwipeAction.FLAG -> (if (message.isFlagged) MessageAction.Unflag else MessageAction.Flag) to false
    }

/**
 * Runs a swipe's [action] on [messageId] through the core (never by changing the list alone). Returns the
 * notice to show if the core refused it, else null.
 */
internal suspend fun runSwipeAction(
    core: MailboxCore,
    messageId: Long,
    action: MessageAction,
    undoable: Boolean,
): Int? =
    try {
        core.swipeAction(messageId, action, undoable)
        null
    } catch (_: ActionFailure.NoArchiveFolder) {
        R.string.action_no_archive_folder
    } catch (_: ActionFailure.Waking) {
        // Coming back from snooze (#504): said as such, not as a generic failure.
        R.string.action_waking
    } catch (_: ActionFailure) {
        R.string.action_not_applied
    }
