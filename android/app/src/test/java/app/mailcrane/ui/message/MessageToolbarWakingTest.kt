package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.foundation.layout.Row
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.FolderSummary
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** While a message comes back from snooze, its toolbar doesn't offer what the core would refuse (#431, #504). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class MessageToolbarWakingTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(waking: Boolean) {
        compose.setContent {
            MailcraneTheme {
                Row {
                    MessageToolbarActions(
                        flagged = false,
                        moveTargets = listOf(FolderSummary(9, "Projects", null, 0u, 0u)),
                        actions = MessageActions(onSnooze = {}),
                        waking = waking,
                    )
                }
            }
        }
    }

    @Test
    fun aWakingMessageCantBeArchivedDeletedMovedOrSnoozed() {
        show(waking = true)
        compose.onNodeWithContentDescription("Archive").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Delete").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Snooze").assertIsNotEnabled()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithText("Move to…").assertIsNotEnabled()
        // What doesn't leave the folder still works.
        compose.onNodeWithContentDescription("Mark as unread").assertIsEnabled()
    }

    @Test
    fun onceItLandsEverythingIsOfferedAgain() {
        show(waking = false)
        compose.onNodeWithContentDescription("Archive").assertIsEnabled()
        compose.onNodeWithContentDescription("Delete").assertIsEnabled()
        compose.onNodeWithContentDescription("Snooze").assertIsEnabled()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithText("Move to…").assertIsEnabled()
    }
}
