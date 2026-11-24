package app.mailcrane.ui.compose

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.RecipientProblem
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
        compose.onNodeWithText("Send").assertIsNotEnabled()
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
        compose.onNodeWithText("Send").assertIsEnabled().performClick()
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
}
