package app.mailcrane.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.mailcrane.R

/**
 * How the mailbox is laid out (M4-13), by window width as in Material's window size classes: compact
 * (under 600 dp) one pane; medium (600–840 dp) the list and the message side by side; expanded (840 dp
 * and wider) the folders always visible too. Built on the window width rather than an extra library.
 */
enum class MailLayout { ONE_PANE, TWO_PANES, THREE_PANES }

/** Widths where the layout changes. */
val MEDIUM_WIDTH = 600.dp
val EXPANDED_WIDTH = 840.dp

fun mailLayoutFor(width: Dp): MailLayout =
    when {
        width >= EXPANDED_WIDTH -> MailLayout.THREE_PANES
        width >= MEDIUM_WIDTH -> MailLayout.TWO_PANES
        else -> MailLayout.ONE_PANE
    }

/** The list, and beside it in two- and three-pane layouts the message. (The folders are the drawer's.) */
@Composable
fun MailPanes(
    layout: MailLayout,
    list: @Composable () -> Unit,
    detail: @Composable () -> Unit,
) {
    if (layout == MailLayout.ONE_PANE) {
        Box(Modifier.fillMaxSize().testTag("list-pane")) { list() }
        return
    }
    // The panes paint the theme's surface: the window behind them is the platform's dark
    // Theme.Material, which showed through the empty reading pane in light mode (#335).
    Row(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface)) {
        Box(Modifier.weight(0.42f).fillMaxHeight().testTag("list-pane")) { list() }
        VerticalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Box(Modifier.weight(0.58f).fillMaxHeight().testTag("detail-pane")) { detail() }
    }
}

/** The detail pane before a message is chosen. */
@Composable
fun NoMessageSelected() {
    Box(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).testTag("no-message"),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = stringResource(R.string.no_message_selected),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * Mailbox shortcuts with a hardware keyboard: Ctrl/⌘+N new message, J or ↓ next message, K or ↑
 * previous. Key events typed into a text field are used by the field first, so letters never trigger
 * these while writing.
 */
fun Modifier.mailShortcuts(
    onCompose: (() -> Unit)?,
    onNext: () -> Unit,
    onPrevious: () -> Unit,
): Modifier =
    onKeyEvent { e ->
        if (e.type != KeyEventType.KeyDown) return@onKeyEvent false
        val command = e.isCtrlPressed || e.isMetaPressed
        when {
            command && e.key == Key.N && onCompose != null -> {
                onCompose()
                true
            }
            !command && (e.key == Key.J || e.key == Key.DirectionDown) -> {
                onNext()
                true
            }
            !command && (e.key == Key.K || e.key == Key.DirectionUp) -> {
                onPrevious()
                true
            }
            else -> false
        }
    }
