package app.mailcrane.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.MessageSummary
import app.mailcrane.ui.messagelist.MessageRow
import app.mailcrane.ui.messagelist.SwipeableRow
import app.mailcrane.ui.messagelist.swipeA11yLabel
import java.time.ZonedDateTime

/** The tag of Settings' sample row, for tests. */
internal const val SWIPE_PREVIEW_TAG = "swipe-preview"

/**
 * Settings' swipe section (#553): a made-up message row the user swipes to try each direction. A swipe runs
 * no mail action: the row springs back and the picker for that direction opens; picking saves the setting
 * and the sample shows it at once. A direction set to "none" can't be swiped, as in the list, so the two
 * pickers below stay too, for that and for TalkBack and keyboards.
 */
@Composable
internal fun SwipePreviewSection(swipe: SwipeSettings) {
    // Which direction's picker is open: true for right (start to end), false for left, null for none.
    var picking by rememberSaveable { mutableStateOf<Boolean?>(null) }
    val resources = LocalResources.current
    val locale = LocalConfiguration.current.locales[0]
    val sample =
        sampleMessage(
            sender = resources.getString(R.string.swipe_preview_sender),
            subject = resources.getString(R.string.swipe_preview_subject),
        )
    Column(Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.swipe_preview_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        SwipeableRow(
            right = swipe.right,
            left = swipe.left,
            a11yLabel = { swipeA11yLabel(it, sample) },
            onSwipe = {},
            onSwipeToward = { startToEnd -> picking = startToEnd },
            directionLabel = { startToEnd ->
                val action = resources.getString(swipeLabel(if (startToEnd) swipe.right else swipe.left))
                val label = if (startToEnd) R.string.swipe_preview_right else R.string.swipe_preview_left
                resources.getString(label, action)
            },
        ) {
            MessageRow(
                message = sample,
                now = ZonedDateTime.now(),
                locale = locale,
                modifier = Modifier.testTag(SWIPE_PREVIEW_TAG),
            )
        }
    }
    picking?.let { startToEnd ->
        SwipePicker(
            title = if (startToEnd) R.string.settings_swipe_right else R.string.settings_swipe_left,
            current = if (startToEnd) swipe.right else swipe.left,
            onPick = {
                picking = null
                if (startToEnd) swipe.onRightChange(it) else swipe.onLeftChange(it)
            },
            onDismiss = { picking = null },
        )
    }
}

/** Picks one direction's swipe action. */
@Composable
private fun SwipePicker(
    title: Int,
    current: SwipeAction,
    onPick: (SwipeAction) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(title)) },
        text = {
            Column(Modifier.selectableGroup()) {
                for (action in SwipeAction.entries) {
                    ListItem(
                        leadingContent = { RadioButton(selected = current == action, onClick = null) },
                        modifier =
                            Modifier.selectable(
                                selected = current == action,
                                role = Role.RadioButton,
                                onClick = { onPick(action) },
                            ),
                    ) {
                        Text(stringResource(swipeLabel(action)))
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.settings_swipe_cancel)) } },
    )
}

/** A made-up message for the sample row: an example.com sender, never real mail. */
private fun sampleMessage(
    sender: String,
    subject: String,
) = MessageSummary(
    id = -1,
    fromName = sender,
    fromAddr = "sample@example.com",
    subject = subject,
    snippet = "",
    date = ZonedDateTime.now().toEpochSecond(),
    isSeen = true,
    isFlagged = false,
    hasAttachments = false,
)
