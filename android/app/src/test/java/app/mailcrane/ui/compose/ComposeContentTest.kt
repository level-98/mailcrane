package app.mailcrane.ui.compose

import android.app.Application
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.RecipientProblem
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class ComposeContentTest {
    @get:Rule
    val compose = createComposeRule()

    private val one = listOf(AccountSummary(1, "ann@example.org", "Ann"))

    private fun show(
        state: ComposeUiState,
        actions: ComposeActions = ComposeActions(),
    ) = compose.setContent { MailcraneTheme { ComposeContent(state, actions) } }

    @Test
    fun sendIsDisabledWithoutAValidRecipient() {
        val invalid = RecipientChip("nope", "", "", RecipientProblem.MISSING_AT)
        show(ComposeUiState(accounts = one, fromAccountId = 1, to = RecipientFieldState(chips = listOf(invalid))))
        compose.onNodeWithContentDescription("Send").assertIsNotEnabled()
        // Invalid chips are announced as such, not only coloured.
        compose.onNodeWithContentDescription("nope, not a valid email address").assertIsDisplayed()
    }

    @Test
    fun sendIsEnabledWithAValidRecipient() {
        var sent = 0
        val chip = RecipientChip("Bob <bob@example.net>", "Bob", "bob@example.net", null)
        show(
            ComposeUiState(accounts = one, fromAccountId = 1, to = RecipientFieldState(chips = listOf(chip))),
            ComposeActions(onSend = { sent++ }),
        )
        compose.onNodeWithContentDescription("Send").assertIsEnabled().performClick()
        assertEquals(1, sent)
    }

    @Test
    fun fromPickerAndCcBccAreHiddenUntilNeeded() {
        show(ComposeUiState(accounts = one, fromAccountId = 1))
        compose.onNodeWithText("From").assertDoesNotExist()
        compose.onNodeWithText("Cc").assertDoesNotExist()
        compose.onNodeWithText("Cc/Bcc").assertIsDisplayed()
    }

    @Test
    fun fromPickerShowsWithSeveralAccounts() {
        val two = one + AccountSummary(2, "ann@example.net", "")
        show(ComposeUiState(accounts = two, fromAccountId = 2, showCcBcc = true))
        compose.onNodeWithText("From").assertIsDisplayed()
        compose.onNodeWithText("ann@example.net").assertIsDisplayed()
        compose.onNodeWithText("Bcc").assertIsDisplayed()
    }

    @Test
    fun discardDialogAsksBeforeLosingContent() {
        var discarded = 0
        var kept = 0
        show(
            ComposeUiState(accounts = one, fromAccountId = 1, body = "Draft", confirmingDiscard = true),
            ComposeActions(onConfirmDiscard = { discarded++ }, onKeepEditing = { kept++ }),
        )
        compose.onNodeWithText("Discard draft?").assertIsDisplayed()
        compose.onNodeWithText("Keep editing").performClick()
        compose.onNodeWithText("Discard").performClick()
        assertEquals(1, kept)
        assertEquals(1, discarded)
    }

    @Test
    fun failureShowsTheErrorAndKeepsTheMessage() {
        show(ComposeUiState(accounts = one, fromAccountId = 1, body = "Still here", error = ComposeError.TIMEOUT))
        compose.onNodeWithText("The outgoing mail server took too long to answer.").assertIsDisplayed()
        compose.onNodeWithText("Your message is still here.").assertIsDisplayed()
        compose.onNodeWithText("Still here").assertIsDisplayed()
    }

    /** #340: on a phone-width bar the title stays on one line, and the moved actions are in the menu. */
    @Test
    // A small phone; Robolectric's default screen is narrower than any current phone.
    @Config(qualifiers = "w360dp-h800dp")
    // Real font measurement, so "fits on one line" means what it would on a phone.
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun titleStaysOnOneLineAndMoreOptionsHoldsScheduleAndDiscard() {
        var discarded = 0
        val chip = RecipientChip("Bob <bob@example.net>", "Bob", "bob@example.net", null)
        compose.setContent {
            MailcraneTheme {
                // A small phone: 360 dp wide.
                Box(Modifier.width(360.dp)) {
                    ComposeContent(
                        ComposeUiState(
                            accounts = one,
                            fromAccountId = 1,
                            to = RecipientFieldState(chips = listOf(chip)),
                        ),
                        ComposeActions(onDiscard = { discarded++ }),
                    )
                }
            }
        }
        val title = compose.onNodeWithText("New message").fetchSemanticsNode()
        val layout = mutableListOf<TextLayoutResult>()
        title.config[SemanticsActions.GetTextLayoutResult].action?.invoke(layout)
        assertEquals("one line", 1, layout.single().lineCount)
        assertFalse("not shortened at the default size", layout.single().isLineEllipsized(0))

        compose.onNodeWithContentDescription("Discard").assertDoesNotExist()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithText("Schedule send").assertIsEnabled()
        compose.onNodeWithText("Discard").performClick()
        assertEquals(1, discarded)
    }

    @Test
    fun theMessageWaitsWhileADraftOrReplyLoads() {
        // #435: read-only with an announced loading indicator, then editable once the load lands.
        var state by mutableStateOf(
            ComposeUiState(accounts = one, fromAccountId = 1, loadingContent = true),
        )
        compose.setContent { MailcraneTheme { ComposeContent(state, ComposeActions()) } }
        compose.onNodeWithTag("compose-body").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Loading the message…").assertIsDisplayed()
        state = state.copy(loadingContent = false)
        compose.waitForIdle()
        compose.onNodeWithTag("compose-body").assertIsEnabled()
        assertEquals(0, compose.onAllNodesWithTag("compose-loading").fetchSemanticsNodes().size)
    }
}
