package app.mailcrane.settings

import android.app.Application
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import app.mailcrane.core.AccountSummary
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
class StartViewSettingsTest {
    @get:Rule
    val compose = createComposeRule()

    private val ann = AccountSummary(1, "ann@example.org", "Ann")
    private val bob = AccountSummary(2, "bob@example.com", "Bob")

    @Test
    fun storedValuesRoundTripAndOddOnesMeanLastUsed() {
        for (v in listOf(StartView.Unified, StartView.Last, StartView.Account(42))) {
            assertEquals(v, StartView.decode(StartView.encode(v)))
        }
        for (odd in listOf(null, "", "account:", "account:x", "account:-3", "account:0", "inbox")) {
            assertEquals(odd, StartView.Last, StartView.decode(odd))
        }
    }

    @Test
    fun theUnifiedInboxIsOfferedOnlyWithTwoAccounts() {
        assertEquals(listOf(StartView.Account(1), StartView.Last), startViewChoices(listOf(ann)))
        assertEquals(
            listOf(StartView.Unified, StartView.Account(1), StartView.Account(2), StartView.Last),
            startViewChoices(listOf(ann, bob)),
        )
    }

    @Test
    fun picksAChoice() {
        var picked: StartView = StartView.Last
        compose.setContent {
            MailcraneTheme {
                // Scrollable, as in Settings, in case the rows don't fit the test screen.
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    StartViewContent(listOf(ann, bob), picked) { picked = it }
                }
            }
        }
        compose.onNodeWithText("Last used").performScrollTo().assertIsSelected()
        compose.onNodeWithText("bob@example.com inbox").performScrollTo().performClick()
        assertEquals(StartView.Account(2), picked)
        compose.onNodeWithText("All inboxes").performScrollTo().performClick()
        assertEquals(StartView.Unified, picked)
    }

    @Test
    fun aRemovedAccountShowsAsLastUsed() {
        compose.setContent { MailcraneTheme { StartViewContent(listOf(ann), StartView.Account(2)) {} } }
        compose.onNodeWithText("Last used").assertIsSelected()
    }
}
