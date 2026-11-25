package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.UnsubscribeInfo
import app.mailcrane.core.UnsubscribeMailto
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class UnsubscribeDialogTest {
    @get:Rule
    val compose = createComposeRule()

    private val info =
        UnsubscribeInfo(
            sender = "news@example.net",
            oneClick = true,
            mailto = UnsubscribeMailto("unsub@example.net", null, null),
            webUrl = "https://lists.example.net/u/1",
        )

    @Test
    fun nothingHappensUntilTheUserConfirms() {
        var confirmed = 0
        compose.setContent { MailcraneTheme { UnsubscribeAction(info, busy = false, onConfirm = { confirmed++ }) } }
        compose.onNodeWithText("Unsubscribe").performClick()
        compose.onNodeWithText("Unsubscribe from news@example.net?").assertIsDisplayed()
        compose.onNodeWithText("lists.example.net", substring = true).assertIsDisplayed()
        assertEquals(0, confirmed)
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, confirmed)
        compose.onNodeWithText("Unsubscribe").performClick()
        compose
            .onAllNodesWithText("Unsubscribe")
            .fetchSemanticsNodes()
            .size
            .let { assertEquals(2, it) }
        compose.onAllNodesWithText("Unsubscribe")[1].performClick()
        assertEquals(1, confirmed)
    }

    @Test
    fun theDialogSaysWhichWayIsUsed() {
        compose.setContent {
            MailcraneTheme {
                UnsubscribeDialog(info.copy(oneClick = false), UnsubscribeMethod.EMAIL, onConfirm = {}, onDismiss = {})
            }
        }
        compose.onNodeWithText("prepare an email to unsub@example.net", substring = true).assertIsDisplayed()
    }

    @Test
    fun theWebWayNamesTheSite() {
        compose.setContent {
            MailcraneTheme {
                UnsubscribeDialog(info, UnsubscribeMethod.WEB, onConfirm = {}, onDismiss = {})
            }
        }
        compose.onNodeWithText("opens lists.example.net in your browser", substring = true).assertIsDisplayed()
    }
}
