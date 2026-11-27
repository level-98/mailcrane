package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.StateRestorationTester
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

/** Swiping rows in the message list (#341, #368), against a state held here instead of a view model. */
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

    private fun show(
        right: SwipeAction = SwipeAction.FLAG,
        onSwipe: (MessageSummary, SwipeAction) -> Unit = { message, action -> swiped += message.id to action },
    ) {
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
                            right = right,
                            left = SwipeAction.ARCHIVE,
                            onSwipe = onSwipe,
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

    @Test
    fun aReadSwipeRunsOnceWhenTheCoreUpdatesTheRow() {
        // Like the core: marking a message unread changes the row while it springs back. That
        // recomposition used to run the action again and again, flipping read and unread, and the
        // row crawled back over seconds.
        show(right = SwipeAction.READ_UNREAD) { message, action ->
            swiped += message.id to action
            state =
                state.copy(
                    messages = state.messages.map { if (it.id == message.id) it.copy(isSeen = !it.isSeen) else it },
                )
        }
        val start = left("Subject 1")
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Subject 1", useUnmergedTree = true).performTouchInput { swipeRight() }
        compose.mainClock.advanceTimeBy(1_000)
        assertEquals(listOf(1L to SwipeAction.READ_UNREAD), swiped)
        assertFalse("marked unread once", state.messages.first { it.id == 1L }.isSeen)
        assertEquals("back in place within a second", start, left("Subject 1"), 1f)
    }

    @Test
    fun aFlagSwipeRunsOnceWhenTheCoreUpdatesTheRow() {
        // The same as a read swipe: flagging changes the row while it springs back.
        show(right = SwipeAction.FLAG) { message, action ->
            swiped += message.id to action
            state =
                state.copy(
                    messages =
                        state.messages.map {
                            if (it.id == message.id) it.copy(isFlagged = !it.isFlagged) else it
                        },
                )
        }
        val start = left("Subject 1")
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Subject 1", useUnmergedTree = true).performTouchInput { swipeRight() }
        compose.mainClock.advanceTimeBy(1_000)
        assertEquals(listOf(1L to SwipeAction.FLAG), swiped)
        assertTrue("flagged once", state.messages.first { it.id == 1L }.isFlagged)
        assertEquals("back in place within a second", start, left("Subject 1"), 1f)
    }

    @Test
    fun anArchiveIsSentOnceWhileTheListChangesDuringTheWait() {
        // A sync or the minute tick redraws the list while an archived row waits to be dropped. Each redraw
        // used to send the archive again.
        show()
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Subject 2", useUnmergedTree = true).performTouchInput { swipeLeft() }
        repeat(3) {
            compose.mainClock.advanceTimeBy(500)
            state =
                state.copy(
                    messages = state.messages.map { if (it.id == 3L) it.copy(isSeen = !it.isSeen) else it },
                )
        }
        compose.mainClock.advanceTimeBy(500)
        assertEquals(listOf(2L to SwipeAction.ARCHIVE), swiped)
    }

    @Test
    fun aSwipeWhileTheRowSpringsBackIsIgnored() {
        // A second swipe that caught the row on its way back used to leave it swiped away for good: the row
        // settled at the same value again, so no action ran and nothing brought it back.
        show(right = SwipeAction.READ_UNREAD)
        val start = left("Subject 1")
        val row = compose.onNodeWithText("Subject 1", useUnmergedTree = true)
        compose.mainClock.autoAdvance = false
        row.performTouchInput { swipeRight() }
        // Step frame by frame until the row is out and moving back towards rest.
        var previous = left("Subject 1")
        var returning = false
        var frames = 0
        while (!returning && frames < 240) {
            compose.mainClock.advanceTimeByFrame()
            val x = left("Subject 1")
            returning = x > start + 20f && x < previous
            previous = x
            frames++
        }
        assertTrue("caught the row springing back", returning)
        row.performTouchInput { swipeRight() }
        compose.mainClock.advanceTimeBy(2_000)
        assertEquals("the second swipe did nothing", listOf(1L to SwipeAction.READ_UNREAD), swiped)
        assertEquals("at rest", start, left("Subject 1"), 1f)
    }

    /**
     * #466: a row restored while swiped away (rotation or process death during an archive's wait) comes
     * back at rest and doesn't send its action again.
     */
    @Test
    fun aRowRestoredMidSwipeDoesNotRunItsActionAgain() {
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
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
        val start = left("Subject 2")
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Subject 2", useUnmergedTree = true).performTouchInput { swipeLeft() }
        // Swiped away, waiting for the list to drop the row: saved and restored in that state.
        compose.mainClock.advanceTimeBy(1_000)
        assertEquals(listOf(2L to SwipeAction.ARCHIVE), swiped)
        // The clock must run for the restore to take the content out and put it back; paused, the old row
        // would just carry on and nothing would be restored.
        compose.mainClock.autoAdvance = true
        restoration.emulateSavedInstanceStateRestore()
        compose.waitForIdle()
        assertEquals("the archive was sent once", listOf(2L to SwipeAction.ARCHIVE), swiped)
        assertEquals("the restored row is at rest", start, left("Subject 2"), 1f)
    }

    /**
     * #467 (closed as not a bug): a row's pending reset ends with the row. Removed while it waits and shown
     * again, the row comes back at rest, and nothing runs twice.
     */
    @Test
    fun aRowRemovedWhileItWaitsComesBackAtRest() {
        show()
        val start = left("Subject 2")
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Subject 2", useUnmergedTree = true).performTouchInput { swipeLeft() }
        compose.mainClock.advanceTimeBy(1_000)
        val all = state.messages
        state = state.copy(messages = all.filter { it.id != 2L })
        compose.mainClock.advanceTimeBy(1_000)
        state = state.copy(messages = all)
        compose.mainClock.advanceTimeBy(SWIPE_REMOVE_TIMEOUT_MS + 1_000)
        assertEquals(listOf(2L to SwipeAction.ARCHIVE), swiped)
        assertEquals("back at rest", start, left("Subject 2"), 1f)
    }
}
