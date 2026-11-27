package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.Density
import androidx.test.core.app.ApplicationProvider
import app.mailcrane.R
import app.mailcrane.core.MessageSummary
import app.mailcrane.ui.theme.MailcraneTheme
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZonedDateTime

/** At the end of the list, the last row scrolls clear of the compose button (#464). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class FabClearanceUiTest {
    @get:Rule
    val compose = createComposeRule()

    private val now = ZonedDateTime.now().toEpochSecond()
    private val count = 30L

    private fun message(id: Long) =
        MessageSummary(
            id = id,
            fromName = "Sender $id",
            fromAddr = "sender$id@example.org",
            // The oldest message comes last; its subject is unique so the test can find its row.
            subject = if (id == count) "Final row" else "Subject $id",
            snippet = "",
            date = now - id * 60,
            isSeen = true,
            isFlagged = true,
            hasAttachments = true,
        )

    private val state = MessageListState(messages = (1..count).map(::message), isLoading = false)

    private fun showScrolledToEnd(
        fontScale: Float,
        selecting: Boolean = false,
    ) {
        val listState = LazyListState()
        val shown = if (selecting) state.copy(selectedMessages = setOf(1L)) else state
        compose.setContent {
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, fontScale)) {
                MailcraneTheme {
                    MessageListContent(
                        state = shown,
                        title = "Inbox",
                        snackbar = SnackbarHostState(),
                        onRefresh = {},
                        onLoadMore = {},
                        listState = listState,
                        onCompose = {},
                    )
                }
            }
        }
        compose.waitForIdle()
        // Far past the last item: the list stops at its real end, end padding included.
        compose.runOnIdle {
            runBlocking { listState.scrollToItem(listState.layoutInfo.totalItemsCount - 1, 100_000) }
        }
        compose.waitForIdle()
    }

    private fun lastRowBottom(): Float =
        compose
            .onNode(hasContentDescription("Final row", substring = true))
            .getBoundsInRoot()
            .bottom
            .value

    private fun fabTop(): Float {
        val label = ApplicationProvider.getApplicationContext<Application>().getString(R.string.compose_fab)
        return compose
            .onNode(hasContentDescription(label) and hasClickAction())
            .getBoundsInRoot()
            .top
            .value
    }

    @Test
    fun theLastRowEndsAboveTheComposeButton() {
        showScrolledToEnd(fontScale = 1f)
        val bottom = lastRowBottom()
        val top = fabTop()
        assertTrue("last row ends at $bottom dp, compose button starts at $top dp", bottom <= top)
    }

    @Test
    fun theLastRowEndsAboveTheComposeButtonAtLargeText() {
        showScrolledToEnd(fontScale = 2f)
        val bottom = lastRowBottom()
        val top = fabTop()
        assertTrue("last row ends at $bottom dp, compose button starts at $top dp", bottom <= top)
    }

    @Test
    fun whileSelectingThereIsNoButtonAndNoGapAtTheEnd() {
        showScrolledToEnd(fontScale = 1f, selecting = true)
        val label = ApplicationProvider.getApplicationContext<Application>().getString(R.string.compose_fab)
        compose.onNode(hasContentDescription(label) and hasClickAction()).assertDoesNotExist()
        // The last row reaches the bottom of the screen, less its divider.
        val rootBottom = compose.onRoot().getBoundsInRoot().bottom.value
        assertTrue("last row ends at ${lastRowBottom()} dp of $rootBottom dp", rootBottom - lastRowBottom() < 4f)
    }
}
