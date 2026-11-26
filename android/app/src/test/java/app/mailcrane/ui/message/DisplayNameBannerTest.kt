package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import app.mailcrane.core.DisplayNameWarning
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class DisplayNameBannerTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun bannerNamesBothAddresses() {
        val warning = DisplayNameWarning(claimed = "ceo@example.com", actual = "attacker@example.net")
        compose.setContent { MailcraneTheme { DisplayNameBanner(warning) } }
        compose.onNodeWithTag("display-name-banner").assertIsDisplayed()
        compose.onNodeWithText("Check who sent this", substring = true, useUnmergedTree = true).assertExists()
        compose
            .onNodeWithText(
                "The name says ceo@example.com, but this message was sent from attacker@example.net.",
                useUnmergedTree = true,
            ).assertExists()
    }

    @Test
    fun nothingWithoutAWarning() {
        compose.setContent { MailcraneTheme { DisplayNameBanner(null) } }
        compose.onNodeWithTag("display-name-banner").assertDoesNotExist()
    }
}
