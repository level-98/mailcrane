package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.AuthResult
import app.mailcrane.core.AuthVerdict
import app.mailcrane.core.SenderCheck
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class SenderAuthTest {
    @get:Rule
    val compose = createComposeRule()

    private fun check(
        verdict: AuthVerdict?,
        spf: AuthResult = AuthResult.NONE,
        dkim: AuthResult = AuthResult.NONE,
        dmarc: AuthResult = AuthResult.NONE,
    ) = SenderCheck(
        trusted = verdict != null,
        verdict = verdict,
        spf = spf,
        spfDomain = "news.example.net",
        dkim = dkim,
        dkimDomain = null,
        dmarc = dmarc,
        dmarcDomain = "example.net",
    )

    @Test
    fun verifiedChipOpensTheSheet() {
        val c =
            check(AuthVerdict.VERIFIED, spf = AuthResult.PASS, dkim = AuthResult.TEMP_ERROR, dmarc = AuthResult.PASS)
        compose.setContent { MailcraneTheme { SenderAuthChip(c) } }
        compose.onNodeWithText("Verified").assertIsDisplayed().performClick()
        compose.onNodeWithTag("sender-auth-sheet").assertExists()
        compose.onNodeWithText("Passed (news.example.net)").assertExists()
        compose.onNodeWithText("Couldn’t be checked right now").assertExists()
        compose.onNodeWithText("Passed (example.net)").assertExists()
        compose.onNodeWithText("SPF", substring = true).assertExists()
        compose.onNodeWithText("DKIM", substring = true).assertExists()
        compose.onNodeWithText("DMARC", substring = true).assertExists()
    }

    @Test
    fun failedAndNotVerifiedWording() {
        compose.setContent { MailcraneTheme { SenderAuthChip(check(AuthVerdict.FAILED, dmarc = AuthResult.FAIL)) } }
        compose.onNodeWithText("Failed").assertIsDisplayed().performClick()
        compose.onNodeWithText("may not be from who it says", substring = true).assertExists()
    }

    @Test
    fun notVerifiedWording() {
        compose.setContent { MailcraneTheme { SenderAuthChip(check(AuthVerdict.NOT_VERIFIED)) } }
        compose.onNodeWithText("Not verified").assertIsDisplayed()
    }

    @Test
    fun noChipWithoutTheProvidersHeader() {
        compose.setContent {
            MailcraneTheme {
                SenderAuthChip(check(null))
                SenderAuthChip(null)
            }
        }
        compose.onNodeWithTag("sender-auth-chip").assertDoesNotExist()
    }
}
