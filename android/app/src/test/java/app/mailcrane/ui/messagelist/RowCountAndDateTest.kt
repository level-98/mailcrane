package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.ThreadSummary
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Locale

/** A conversation's message count under the date (#392), and conversation dates with the time (#399). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class RowCountAndDateTest {
    @get:Rule
    val compose = createComposeRule()

    private val zone = ZoneId.of("Europe/London")
    private val now = ZonedDateTime.of(LocalDateTime.of(2026, 11, 24, 15, 0), zone)

    private fun at(
        day: Int,
        hour: Int,
        minute: Int,
    ) = ZonedDateTime.of(LocalDateTime.of(2026, 11, day, hour, minute), zone).toEpochSecond()

    private val message =
        MessageSummary(
            id = 1,
            fromName = "Ana",
            fromAddr = "ana@example.org",
            subject = "Lunch",
            snippet = "Shall we meet at noon?",
            date = at(24, 10, 42),
            isSeen = true,
            isFlagged = false,
            hasAttachments = false,
        )

    private val thread =
        ThreadSummary(
            threadId = 7,
            // Unlike the plain row's, so the test can find the row that holds this date.
            subject = "Dinner",
            participants = listOf("Ana", "Ben"),
            messageCount = 3u,
            unreadCount = 0u,
            lastDate = at(24, 10, 42),
            newestMessageId = 1,
            isFlagged = false,
            hasAttachments = false,
        )

    @Test
    fun aCountedRowIsAsWideAsAPlainOneSoTheDatesLineUp() {
        compose.setContent {
            MailcraneTheme {
                Column(Modifier.width(360.dp)) {
                    MessageRow(message, now, Locale.UK, modifier = Modifier.testTag("plain"))
                    ThreadRow(thread, message, now, Locale.UK, onClick = {})
                }
            }
        }
        val plain = compose.onNodeWithTag("plain").getBoundsInRoot()
        // The row that draws the thread's date: found by its own description, in the unmerged tree, so
        // before #392 this is the inner row the count squeezed (#481 review), not the full-width wrapper.
        val counted =
            compose
                .onNode(hasContentDescription("Dinner", substring = true), useUnmergedTree = true)
                .getBoundsInRoot()
        // The date sits at that row's end, inside the same padding, so equal rows mean the dates line up.
        assertEquals(plain.left, counted.left)
        assertEquals(plain.right, counted.right)
    }

    @Test
    fun theCountIsReadAsPartOfTheRowNotOnItsOwn() {
        compose.setContent {
            MailcraneTheme { ThreadRow(thread, message, now, Locale.UK, onClick = {}) }
        }
        compose.onAllNodes(hasContentDescription("3 messages")).assertCountEquals(0)
        compose.onAllNodes(hasContentDescription("3 messages.", substring = true)).assertCountEquals(1)
    }

    @Test
    fun aRowWithoutACountReadsAsBefore() {
        compose.setContent {
            MailcraneTheme { MessageRow(message, now, Locale.UK) }
        }
        compose.onAllNodes(hasContentDescription("messages", substring = true)).assertCountEquals(0)
    }

    /** ICU may put a narrow or no-break space before am/pm; tests compare plain spaces. */
    private fun conversationDate(
        epochSeconds: Long,
        locale: Locale,
        is24Hour: Boolean,
    ) = formatConversationDate(epochSeconds, now, locale, is24Hour).replace(' ', ' ').replace(' ', ' ')

    @Test
    fun conversationDatesTodayAreTheTime() {
        assertEquals("10:42", conversationDate(at(24, 10, 42), Locale.UK, is24Hour = true))
        assertEquals("14:05", conversationDate(at(24, 14, 5), Locale.UK, is24Hour = true))
        val twelve = conversationDate(at(24, 14, 5), Locale.UK, is24Hour = false)
        assertTrue(twelve, Regex("""^2:05 ?[pP]\.?[mM]\.?$""").matches(twelve))
    }

    @Test
    fun conversationDatesThisWeekAreTheWeekdayAndTime() {
        val day = conversationDate(at(18, 9, 0), Locale.UK, is24Hour = true) // six days ago
        assertTrue(day, day.startsWith("Wed") && day.endsWith("09:00"))
        val twelve = conversationDate(at(18, 21, 30), Locale.UK, is24Hour = false)
        assertTrue(twelve, twelve.startsWith("Wed") && Regex("""9:30 ?[pP]\.?[mM]\.?$""").containsMatchIn(twelve))
    }

    @Test
    fun olderConversationDatesAreTheShortDateAndTime() {
        val uk = conversationDate(at(17, 9, 0), Locale.UK, is24Hour = true) // a week ago
        assertTrue(uk, uk.startsWith("17/11/2026") && uk.endsWith("09:00"))
        val us = conversationDate(at(17, 9, 0), Locale.US, is24Hour = false)
        assertTrue(us, us.startsWith("11/17/2026") && Regex("""9:00 ?[aA]\.?[mM]\.?$""").containsMatchIn(us))
    }

    /** #509: today's list time follows the device's 12/24-hour setting, not only the locale. */
    @Test
    fun listTimesTodayFollowThe24HourSetting() {
        val listDate = { epoch: Long, locale: Locale, is24: Boolean ->
            formatListDate(epoch, now, locale, is24).replace(' ', ' ').replace(' ', ' ')
        }
        // QA's case: US English (12-hour by default) on a device set to 24-hour.
        assertEquals("14:53", listDate(at(24, 14, 53), Locale.US, true))
        val twelve = listDate(at(24, 14, 53), Locale.US, false)
        assertTrue(twelve, Regex("""^2:53 ?[pP]\.?[mM]\.?$""").matches(twelve))
        // And a 24-hour locale on a 12-hour device.
        val ukTwelve = listDate(at(24, 9, 5), Locale.UK, false)
        assertTrue(ukTwelve, Regex("""^9:05 ?[aA]\.?[mM]\.?$""").matches(ukTwelve))
        // Older dates have no time, so the setting changes nothing there.
        assertEquals("Wed", listDate(at(18, 14, 0), Locale.UK, true))
        assertEquals("17/11/2026", listDate(at(17, 14, 0), Locale.UK, false))
    }

    @Test
    fun hostileConversationDatesAreEmptyAndTheListIsUnchanged() {
        assertEquals("", formatConversationDate(Long.MAX_VALUE, now, Locale.UK, true))
        assertEquals("", formatConversationDate(Long.MIN_VALUE, now, Locale.UK, false))
        // The message list keeps its short dates.
        assertEquals("10:42", formatListDate(at(24, 10, 42), now, Locale.UK))
        assertEquals("Wed", formatListDate(at(18, 9, 0), now, Locale.UK))
    }
}
