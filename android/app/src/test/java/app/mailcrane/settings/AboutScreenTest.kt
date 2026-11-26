package app.mailcrane.settings

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import app.mailcrane.BuildConfig
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class AboutScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val opened = mutableListOf<String>()

    private fun show() {
        compose.setContent {
            MailcraneTheme {
                AboutContent(
                    version = AppVersion(name = "0.1.0-rc.1", code = 100, commit = "81755b8"),
                    onBack = {},
                    onOpen = { opened += it },
                )
            }
        }
    }

    @Test
    fun showsTheVersionAndCommit() {
        show()
        compose.onNodeWithText("Version 0.1.0-rc.1 (100)").assertIsDisplayed()
        compose.onNodeWithText("Built from commit 81755b8").assertIsDisplayed()
    }

    @Test
    fun nothingOpensUntilALinkIsTapped() {
        show()
        assertTrue(opened.isEmpty())
        compose.onNodeWithText("Privacy policy").performScrollTo().performClick()
        assertEquals(listOf("https://mailcrane.app/privacy/"), opened)
    }

    @Test
    fun eachRowOpensItsPage() {
        show()
        for (title in listOf("Website", "Privacy policy", "Source code", "Licence")) {
            compose.onNodeWithText(title).performScrollTo().performClick()
        }
        assertEquals(
            listOf(AboutLinks.WEBSITE, AboutLinks.PRIVACY, AboutLinks.SOURCE, AboutLinks.LICENCE),
            opened,
        )
    }

    @Test
    fun linksGoOnlyToTheProjectsOwnSitesOverHttps() {
        val allowed = setOf("mailcrane.app", "github.com")
        for (url in listOf(AboutLinks.WEBSITE, AboutLinks.PRIVACY, AboutLinks.SOURCE, AboutLinks.LICENCE)) {
            val uri = android.net.Uri.parse(url)
            assertEquals(url, "https", uri.scheme)
            assertTrue(url, uri.host in allowed)
        }
    }

    @Test
    fun theBuildRecordsACommitOrSaysUnknown() {
        val commit = BuildConfig.GIT_COMMIT
        assertTrue(commit, commit == "unknown" || Regex("[0-9a-f]{7}").matches(commit))
    }
}
