package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * #511: the toolbar's read button says what it does. Unread, it is "Mark as read" (an open envelope); once read,
 * as when a delayed mark-read lands, it turns into "Mark as unread" (the closed one).
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class MarkReadButtonTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theButtonTurnsIntoMarkAsUnreadOnceTheMessageIsRead() {
        val tapped = mutableListOf<String>()
        var unread by mutableStateOf(true)
        compose.setContent {
            MailcraneTheme {
                Row {
                    MessageToolbarActions(
                        flagged = false,
                        moveTargets = emptyList(),
                        actions =
                            MessageActions(
                                onMarkUnread = { tapped += "unread" },
                                onMarkRead = if (unread) ({ tapped += "read" }) else null,
                            ),
                    )
                }
            }
        }
        compose.onNodeWithContentDescription("Mark as read").assertIsDisplayed().performClick()
        compose.onNodeWithContentDescription("Mark as unread").assertDoesNotExist()

        unread = false
        compose.onNodeWithContentDescription("Mark as unread").assertIsDisplayed().performClick()
        compose.onNodeWithContentDescription("Mark as read").assertDoesNotExist()
        assertEquals(listOf("read", "unread"), tapped)
    }
}
