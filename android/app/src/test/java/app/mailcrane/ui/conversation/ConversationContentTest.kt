package app.mailcrane.ui.conversation

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.ThreadSummary
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
}
