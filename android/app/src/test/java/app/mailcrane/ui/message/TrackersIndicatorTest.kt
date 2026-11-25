package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.TrackerInfo
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class TrackersIndicatorTest {
    @get:Rule
    val compose = createComposeRule()

    private val trackers =
        listOf(
            TrackerInfo(service = "Mailchimp", domain = "list-manage.com"),
            TrackerInfo(service = null, domain = "stats.example.net"),
            TrackerInfo(service = null, domain = ""),
        )

    @Test
    fun countIsShownAndTapOpensTheList() {
        compose.setContent { MailcraneTheme { TrackersIndicator(trackers) } }
        compose.onNodeWithText("3 trackers blocked").assertIsDisplayed().performClick()
        compose.onNodeWithTag("trackers-sheet").assertExists()
        compose.onNodeWithText("Mailchimp").assertExists()
        compose.onNodeWithText("list-manage.com").assertExists()
        compose.onNodeWithText("stats.example.net").assertExists()
        compose.onNodeWithText("Embedded in the message").assertExists()
    }

    @Test
    fun singularWording() {
        compose.setContent { MailcraneTheme { TrackersIndicator(trackers.take(1)) } }
        compose.onNodeWithText("1 tracker blocked").assertIsDisplayed()
    }

    @Test
    fun sheetExplainsTrackingPixels() {
        compose.setContent { MailcraneTheme { TrackersSheet(trackers) } }
        compose.onNodeWithText("Trackers blocked").assertIsDisplayed()
        compose.onNodeWithText("A tracking pixel is", substring = true).assertExists()
    }
}
