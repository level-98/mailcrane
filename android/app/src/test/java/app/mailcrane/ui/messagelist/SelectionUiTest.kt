package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import app.mailcrane.core.MessageSummary
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZonedDateTime

/** Multi-select (M4-14) on the message list, against a state held here instead of a view model. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class SelectionUiTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val now = ZonedDateTime.now().toEpochSecond()

    private fun message(id: Long) =
        MessageSummary(
            id = id,
            fromName = "Sender $id",
            fromAddr = "sender$id@example.org",
            subject = "Subject $id",
            snippet = "",
            date = now - id,
            isSeen = id != 1L,
            isFlagged = false,
            hasAttachments = false,
        )

    private var state by mutableStateOf(
        MessageListState(messages = listOf(message(1), message(2), message(3)), isLoading = false),
    )
    private val actions = mutableListOf<BulkAction>()
    private val opened = mutableListOf<Long>()

    private fun toggle(id: Long) {
        val selected = state.selectedMessages
        state = state.copy(selectedMessages = if (id in selected) selected - id else selected + id)
    }

    private fun show() {
        compose.setContent {
            MailcraneTheme {
                MessageListContent(
                    state = state,
                    title = "Inbox",
                    snackbar = SnackbarHostState(),
                    onRefresh = {},
                    onLoadMore = {},
                    onOpenMessage = { opened += it },
                    selection =
                        SelectionHandlers(
                            onToggleMessage = ::toggle,
                            onClear = { state = state.copy(selectedMessages = emptySet()) },
                            onAction = { actions += it },
                        ),
                )
            }
        }
    }

    private fun row(id: Long) = compose.onNodeWithContentDescription("From Sender $id", substring = true)

    @Test
    fun longPressStartsSelectionAndTheBarCounts() {
        show()
        compose.onAllNodesWithTag("selection-bar").assertCountEquals(0)
        row(2).performTouchInput { longClick() }
        compose.onNodeWithTag("selection-bar").assertExists()
        compose.onNodeWithText("1 selected").assertExists()
        // While selecting, a tap selects instead of opening.
        row(3).performClick()
        compose.onNodeWithText("2 selected").assertExists()
        assertEquals(emptyList<Long>(), opened)
        compose
            .onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.Selected, true))
            .assertCountEquals(2)
        row(3).performClick()
        compose.onNodeWithText("1 selected").assertExists()
    }

    @Test
    fun theBarOffersEveryActionAndClearEndsSelection() {
        show()
        row(2).performTouchInput { longClick() }
        // Message 2 is read, so the bar offers Mark as unread.
        compose.onNodeWithContentDescription("Mark as unread").performClick()
        compose.onNodeWithContentDescription("Flag").performClick()
        compose.onNodeWithContentDescription("Archive").performClick()
        compose.onNodeWithContentDescription("Delete").performClick()
        assertEquals(listOf(BulkAction.UNREAD, BulkAction.FLAG, BulkAction.ARCHIVE, BulkAction.DELETE), actions)
        compose.onNodeWithContentDescription("Move to…").assertExists()
        // With the unread message 1 selected too, it offers Mark as read.
        row(1).performClick()
        compose.onNodeWithContentDescription("Mark as read").assertExists()
        compose.onNodeWithContentDescription("Clear selection").performClick()
        compose.onAllNodesWithTag("selection-bar").assertCountEquals(0)
    }

    @Test
    fun backEndsSelectionFirst() {
        show()
        row(2).performTouchInput { longClick() }
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.onAllNodesWithTag("selection-bar").assertCountEquals(0)
        assertEquals(emptySet<Long>(), state.selectedMessages)
    }

    @Test
    fun tapOpensWhenNotSelecting() {
        show()
        row(3).performClick()
        assertEquals(listOf(3L), opened)
    }

    @Test
    fun thePermanentDeleteDialogNamesTheCountAndConfirms() {
        var confirmed = false
        compose.setContent {
            MailcraneTheme { DeleteForGoodDialog(count = 2, onConfirm = { confirmed = true }, onDismiss = {}) }
        }
        compose.onNodeWithText("Delete 2 messages for good?").assertExists()
        compose.onNodeWithText("Delete for good").performClick()
        assertEquals(true, confirmed)
    }
}
