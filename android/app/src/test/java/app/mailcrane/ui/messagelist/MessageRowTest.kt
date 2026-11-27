package app.mailcrane.ui.messagelist

import android.app.Application
import android.provider.Settings
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.test.core.app.ApplicationProvider
import app.mailcrane.core.MessageSummary
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Locale

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class MessageRowTest {
    @get:Rule
    val compose = createComposeRule()

    // The row's time follows the device's 12/24-hour setting (#509): pin it, so the expected times
    // don't depend on the runner's locale.
    @Before
    fun use24HourClock() {
        Settings.System.putString(
            ApplicationProvider.getApplicationContext<Application>().contentResolver,
            Settings.System.TIME_12_24,
            "24",
        )
    }

    private val zone = ZoneId.of("Europe/London")
    private val now = ZonedDateTime.of(LocalDateTime.of(2026, 11, 24, 15, 0), zone)

    private fun at(
        day: Int,
        hour: Int,
        minute: Int,
    ) = ZonedDateTime.of(LocalDateTime.of(2026, 11, day, hour, minute), zone).toEpochSecond()

    private val lunch =
        MessageSummary(
            id = 1,
            fromName = "Ana",
            fromAddr = "ana@example.org",
            subject = "Lunch",
            snippet = "Shall we meet at noon?",
            date = at(24, 10, 42),
            isSeen = false,
            isFlagged = false,
            hasAttachments = true,
        )

    private fun show(message: MessageSummary) {
        compose.setContent {
            MailcraneTheme { MessageRow(message = message, now = now, locale = Locale.UK) }
        }
    }

    @Test
    fun rowIsOneTalkBackItemWithEveryState() {
        show(lunch)
        compose
            .onNodeWithContentDescription("Unread. From Ana. Subject: Lunch. 10:42. Has attachment.")
            .assertExists()
        // The inner texts and icons are not separate items.
        compose
            .onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.ContentDescription))
            .assertCountEquals(1)
        compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text)).assertCountEquals(0)
    }

    @Test
    fun readFlaggedRowWithoutSenderOrSubject() {
        show(
            lunch.copy(
                isSeen = true,
                isFlagged = true,
                hasAttachments = false,
                fromName = "",
                fromAddr = "",
                subject = "",
            ),
        )
        compose
            .onNodeWithContentDescription("Flagged. From Unknown sender. Subject: (No subject). 10:42.")
            .assertExists()
    }

    @Test
    fun senderFallsBackToAddress() {
        show(lunch.copy(fromName = " "))
        compose.onRoot().assertExists()
        compose
            .onNodeWithContentDescription("Unread. From ana@example.org. Subject: Lunch. 10:42. Has attachment.")
            .assertExists()
    }

    @Test
    fun hostileDateStillRenders() {
        show(lunch.copy(date = Long.MIN_VALUE))
        compose
            .onNodeWithContentDescription("Unread. From Ana. Subject: Lunch. Has attachment.")
            .assertExists()
    }

    @Test
    fun relativeDates() {
        val uk = Locale.UK
        assertEquals("10:42", formatListDate(at(24, 10, 42), now, uk))
        assertEquals("Wed", formatListDate(at(18, 9, 0), now, uk)) // six days ago
        assertEquals("17/11/2026", formatListDate(at(17, 9, 0), now, uk)) // a week ago
        assertEquals("25/11/2026", formatListDate(at(25, 9, 0), now, uk)) // clock skew: tomorrow
        assertEquals("", formatListDate(Long.MAX_VALUE, now, uk))
        assertEquals("Mi.", formatListDate(at(18, 9, 0), now, Locale.GERMANY))
    }
}
