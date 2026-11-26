package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import app.mailcrane.core.MessageSummary
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZonedDateTime

/** Swiping rows in the message list (#341), against a state held here instead of a view model. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class SwipeUiTest {
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
            isSeen = true,
            isFlagged = false,
            hasAttachments = false,
        )

    private var state by mutableStateOf(
        MessageListState(messages = listOf(message(1), message(2), message(3)), isLoading = false),
    )
    private val swiped = mutableListOf<Pair<Long, SwipeAction>>()

    private fun show() {
        compose.setContent {
            MailcraneTheme {
                MessageListContent(
                    state = state,
                    title = "Inbox",
                    snackbar = SnackbarHostState(),
                    onRefresh = {},
                    onLoadMore = {},
                    swipe =
                        SwipeHandlers(
                            right = SwipeAction.FLAG,
                            left = SwipeAction.ARCHIVE,
                            onSwipe = { message, action -> swiped += message.id to action },
                        ),
                )
            }
        }
    }

    // Where the row's subject is, unclipped: a row swiped off the screen has a negative x.
    private fun left(subject: String) =
        compose
            .onNodeWithText(subject, useUnmergedTree = true)
            .fetchSemanticsNode()
            .positionInRoot.x

    @Test
    fun onlyArchiveAndDeleteTakeTheRowAway() {
        assertTrue(swipeRemovesRow(SwipeAction.ARCHIVE))
        assertTrue(swipeRemovesRow(SwipeAction.DELETE))
        for (stays in listOf(SwipeAction.READ_UNREAD, SwipeAction.FLAG, SwipeAction.MOVE, SwipeAction.NONE)) {
            assertFalse(stays.name, swipeRemovesRow(stays))
        }
    }

    @Test
    fun anArchivedRowStaysAwayUntilTheListDropsIt() {
        show()
        val start = left("Subject 2")
        compose.onNodeWithText("Subject 2", useUnmergedTree = true).performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(2L to SwipeAction.ARCHIVE), swiped)
        // It didn't spring back.
        assertTrue("still swiped away", left("Subject 2") < start - 100f)

        // The core drops it: the row goes and the others stay.
        state = state.copy(messages = listOf(message(1), message(3)))
        compose.waitForIdle()
        compose.onNodeWithText("Subject 2", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Subject 3", useUnmergedTree = true).assertExists()
    }

    @Test
    fun anArchivedRowTheListKeepsComesBack() {
        show()
        val start = left("Subject 2")
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Subject 2", useUnmergedTree = true).performTouchInput { swipeLeft() }
        compose.mainClock.advanceTimeBy(1_000)
        assertTrue("away while the core works", left("Subject 2") < start - 100f)
        compose.mainClock.advanceTimeBy(SWIPE_REMOVE_TIMEOUT_MS + 1_000)
        assertEquals("back in place", start, left("Subject 2"), 1f)
    }

    @Test
    fun aFlaggedRowSpringsBack() {
        show()
        val start = left("Subject 1")
        compose.onNodeWithText("Subject 1", useUnmergedTree = true).performTouchInput { swipeRight() }
        compose.waitForIdle()
        assertEquals(listOf(1L to SwipeAction.FLAG), swiped)
        assertEquals(start, left("Subject 1"), 1f)
    }
}
