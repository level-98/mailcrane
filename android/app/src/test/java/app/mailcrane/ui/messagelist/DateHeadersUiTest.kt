package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.compose.material3.SnackbarHostState
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithText
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.ThreadSummary
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Locale

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class DateHeadersUiTest {
    @get:Rule
    val compose = createComposeRule()

    // The list reads the real clock, so the rows are dated from it.
    private val now = ZonedDateTime.now()
    private val old =
        LocalDate
            .of(2020, 3, 15)
            .atStartOfDay(ZoneId.systemDefault())
            .plusHours(12)
            .toEpochSecond()

    private fun message(
        id: Long,
        date: Long,
    ) = MessageSummary(
        id = id,
        fromName = "Sender $id",
        fromAddr = "sender$id@example.org",
        subject = "Subject $id",
        snippet = "",
        date = date,
        isSeen = true,
        isFlagged = false,
        hasAttachments = false,
    )

    private val messages =
        listOf(
            message(1, now.toEpochSecond()),
            message(2, now.minusDays(1).toEpochSecond()),
            message(3, old),
            message(4, old - 60),
        )

    private fun show(state: MessageListState) {
        compose.setContent {
            MailcraneTheme {
                MessageListContent(
                    state = state,
                    title = "Inbox",
                    snackbar = SnackbarHostState(),
                    onRefresh = {},
                    onLoadMore = {},
                )
            }
        }
    }

    private val isHeading = SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)

    @Test
    fun messageListHasOneHeadingPerGroup() {
        show(MessageListState(messages = messages, isLoading = false))
        compose.onNodeWithText("Today").assertExists()
        compose.onNodeWithText("Yesterday").assertExists()
        compose.onNodeWithText("March 2020").assertExists()
        compose.onAllNodesWithTag("date-header").assertCountEquals(3)
        compose.onAllNodes(isHeading).assertCountEquals(3)
    }

    @Test
    fun conversationListHasHeadingsToo() {
        val threads =
            listOf(
                ThreadSummary(10, "Plans", listOf("Ana", "Bob"), 3u, 0u, now.toEpochSecond(), false, false, 1),
                ThreadSummary(11, "Old", listOf("Cy"), 2u, 0u, old, false, false, 3),
            )
        show(MessageListState(messages = messages, isLoading = false, grouped = true, threads = threads))
        compose.onNodeWithText("Today").assertExists()
        compose.onNodeWithText("March 2020").assertExists()
        compose.onAllNodes(isHeading).assertCountEquals(2)
    }

    @Test
    fun monthLabelsFollowTheLocale() {
        assertEquals("November", monthLabel(DateGroup.Month(2026, 11), 2026, Locale.UK))
        assertEquals("November 2025", monthLabel(DateGroup.Month(2025, 11), 2026, Locale.UK))
        assertEquals("2025年11月", monthLabel(DateGroup.Month(2025, 11), 2026, Locale.JAPAN))
        assertEquals("Oktober 2025", monthLabel(DateGroup.Month(2025, 10), 2026, Locale.GERMANY))
    }
}
