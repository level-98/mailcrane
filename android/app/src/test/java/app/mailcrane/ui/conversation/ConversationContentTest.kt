package app.mailcrane.ui.conversation

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.ThreadSummary
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.messagelist.SwipeHandlers
import app.mailcrane.ui.messagelist.ThreadRow
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZonedDateTime
import java.util.Locale

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class ConversationContentTest {
    @get:Rule
    val compose = createComposeRule()

    private fun message(
        id: Long,
        from: String,
        subject: String,
    ) = MessageSummary(id, from, "$from@example.net".lowercase(), subject, "Text $id", 1_000 + id, true, false, false)

    @Test
    fun messagesShowInOrderAndOpen() {
        var opened: Long? = null
        compose.setContent {
            MailcraneTheme {
                ConversationContent(
                    title = "Lunch?",
                    messages = listOf(message(1, "Ana", "Lunch?"), message(2, "Bob", "Re: Lunch?")),
                    onBack = {},
                    onOpenMessage = { opened = it },
                )
            }
        }
        // Message rows read as one sentence for screen readers.
        compose.onNodeWithContentDescription("Ana", substring = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Bob", substring = true).assertIsDisplayed().performClick()
        assertEquals(2L, opened)
    }

    /** #399: inside a conversation each message's date includes its time, not only the weekday. */
    @Test
    fun conversationRowsShowTheTime() {
        val zone = java.time.ZoneId.systemDefault()
        val today =
            ZonedDateTime
                .now(zone)
                .withHour(9)
                .withMinute(5)
                .withSecond(0)
        val sent = today.toEpochSecond()
        compose.setContent {
            MailcraneTheme {
                ConversationContent(
                    title = "Lunch?",
                    messages = listOf(message(1, "Ana", "Lunch?").copy(date = sent)),
                    onBack = {},
                    onOpenMessage = {},
                )
            }
        }
        // The row's date reads as a time (9:05 or 09:05, by the 12/24-hour setting).
        compose.onNodeWithContentDescription("9:05", substring = true).assertIsDisplayed()
    }

    @Test
    fun aThreadRowNamesItsPeopleAndCount() {
        var clicked = false
        val thread = ThreadSummary(7, "Plans", listOf("Ana", "Bob"), 3u, 1u, 1_000, false, false, 70)
        compose.setContent {
            MailcraneTheme {
                ThreadRow(thread, null, ZonedDateTime.now(), Locale.UK) { clicked = true }
            }
        }
        compose.onNodeWithContentDescription("Ana, Bob", substring = true).assertIsDisplayed()
        // Read as part of the row since #392, not as a node of its own.
        compose.onNodeWithContentDescription("3 messages.", substring = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Plans", substring = true).performClick()
        assertEquals(true, clicked)
    }

    /** #552: a message inside a conversation swipes as a list row does, with the list's swipe settings. */
    @Test
    fun aMessageInAConversationSwipes() {
        val swiped = mutableListOf<Pair<Long, SwipeAction>>()
        compose.setContent {
            MailcraneTheme {
                ConversationContent(
                    title = "Lunch?",
                    messages = listOf(message(1, "Ana", "Lunch?"), message(2, "Bob", "Re: Lunch?")),
                    onBack = {},
                    onOpenMessage = {},
                    swipe =
                        SwipeHandlers(
                            right = SwipeAction.FLAG,
                            left = SwipeAction.ARCHIVE,
                            onSwipe = { message, action -> swiped += message.id to action },
                        ),
                )
            }
        }
        compose.onNodeWithContentDescription("Bob", substring = true).performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(2L to SwipeAction.ARCHIVE), swiped)
    }

    /** #571: an archived message stays in the conversation, so its row springs back rather than waiting. */
    @Test
    fun anArchivedMessagesRowSpringsBackAtOnce() {
        compose.setContent {
            MailcraneTheme {
                ConversationContent(
                    title = "Lunch?",
                    messages = listOf(message(1, "Ana", "Lunch?"), message(2, "Bob", "Re: Lunch?")),
                    onBack = {},
                    onOpenMessage = {},
                    swipe = SwipeHandlers(right = SwipeAction.FLAG, left = SwipeAction.ARCHIVE),
                )
            }
        }
        val row = compose.onNodeWithContentDescription("Bob", substring = true)
        val start = row.fetchSemanticsNode().positionInRoot.x
        compose.mainClock.autoAdvance = false
        row.performTouchInput { swipeLeft() }
        compose.mainClock.advanceTimeBy(1_000)
        assertEquals("back in place within a second", start, row.fetchSemanticsNode().positionInRoot.x, 1f)
    }
}
