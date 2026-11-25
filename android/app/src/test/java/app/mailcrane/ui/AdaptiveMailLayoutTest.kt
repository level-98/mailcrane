package app.mailcrane.ui

import android.app.Application
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.unit.dp
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
class AdaptiveMailLayoutTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun widthsChooseThePaneCount() {
        assertEquals(MailLayout.ONE_PANE, mailLayoutFor(360.dp))
        assertEquals(MailLayout.ONE_PANE, mailLayoutFor(599.dp))
        assertEquals(MailLayout.TWO_PANES, mailLayoutFor(600.dp))
        assertEquals(MailLayout.TWO_PANES, mailLayoutFor(839.dp))
        assertEquals(MailLayout.THREE_PANES, mailLayoutFor(840.dp))
        assertEquals(MailLayout.THREE_PANES, mailLayoutFor(1280.dp))
    }

    @Test
    fun onePaneShowsOnlyTheList() {
        compose.setContent { MailcraneTheme { MailPanes(MailLayout.ONE_PANE, { Text("LIST") }, { Text("DETAIL") }) } }
        compose.onNodeWithText("LIST").assertIsDisplayed()
        compose.onNodeWithTag("detail-pane").assertDoesNotExist()
    }

    @Test
    fun twoPanesShowListAndDetailSideBySide() {
        compose.setContent {
            MailcraneTheme {
                Box(Modifier.size(900.dp, 600.dp)) {
                    MailPanes(MailLayout.TWO_PANES, { Text("LIST") }, { NoMessageSelected() })
                }
            }
        }
        compose.onNodeWithText("LIST").assertIsDisplayed()
        compose.onNodeWithTag("detail-pane").assertIsDisplayed()
        compose.onNodeWithText("Choose a message to read it here").assertIsDisplayed()
    }

    @OptIn(ExperimentalTestApi::class)
    @Test
    fun keyboardShortcutsMoveAndCompose() {
        var composed = 0
        var moves = 0
        val focus = FocusRequester()
        compose.setContent {
            Box(
                Modifier
                    .size(200.dp)
                    .mailShortcuts({ composed++ }, { moves++ }, { moves-- })
                    .focusRequester(focus)
                    .focusable()
                    .testTag("root"),
            )
        }
        compose.runOnIdle { focus.requestFocus() }
        compose.onNodeWithTag("root").performKeyInput {
            pressKey(Key.J)
            pressKey(Key.J)
            pressKey(Key.K)
            pressKey(Key.DirectionDown)
            withKeyDown(Key.CtrlLeft) { pressKey(Key.N) }
        }
        assertEquals(2, moves)
        assertEquals(1, composed)
    }
}
