package app.mailcrane.ui.messagelist

import android.app.Application
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.click
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import app.mailcrane.core.MessageSummary
import app.mailcrane.settings.MailboxDensity
import app.mailcrane.ui.theme.LocalMailboxDensity
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZonedDateTime
import java.util.Locale

/** Sender pictures and the two densities (M4-24). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class DensityUiTest {
    @get:Rule
    val compose = createComposeRule()

    private val now = ZonedDateTime.now()

    private fun message(
        id: Long,
        accountId: Long = 0,
    ) = MessageSummary(
        id = id,
        fromName = "Ana López",
        fromAddr = "ana$id@example.org",
        subject = "Subject $id",
        snippet = "Preview text $id",
        date = now.toEpochSecond(),
        isSeen = true,
        isFlagged = false,
        hasAttachments = false,
        accountId = accountId,
        accountColor = 1u,
    )

    private fun show(
        density: MailboxDensity,
        selected: Boolean = false,
        onAvatarClick: (() -> Unit)? = null,
        rows: Int = 1,
        accountId: Long = 0,
    ) {
        compose.setContent {
            MailcraneTheme {
                CompositionLocalProvider(LocalMailboxDensity provides density) {
                    Column {
                        repeat(rows) { i ->
                            MessageRow(
                                message = message(i.toLong() + 1, accountId),
                                now = now,
                                locale = Locale.UK,
                                modifier = Modifier.testTag("row-$i"),
                                selecting = selected,
                                selected = selected,
                                onAvatarClick = onAvatarClick,
                            )
                        }
                    }
                }
            }
        }
    }

    @Test
    fun comfortableShowsAPicturePerRowAndCompactNone() {
        show(MailboxDensity.COMFORTABLE, rows = 3)
        compose.onAllNodes(leading(RowLeading.PICTURE)).assertCountEquals(3)
    }

    @Test
    fun compactShowsNoPicture() {
        show(MailboxDensity.COMPACT, rows = 3)
        compose.onAllNodes(leading(RowLeading.PICTURE)).assertCountEquals(0)
        compose.onAllNodes(leading(RowLeading.UNREAD_DOT)).assertCountEquals(3)
    }

    @Test
    fun compactRowsAreShorterSoMoreFit() {
        compose.setContent {
            MailcraneTheme {
                Column {
                    for (density in MailboxDensity.entries) {
                        CompositionLocalProvider(LocalMailboxDensity provides density) {
                            MessageRow(
                                message = message(1),
                                now = now,
                                locale = Locale.UK,
                                modifier = Modifier.testTag("row-${density.name}"),
                            )
                        }
                    }
                }
            }
        }
        val comfortable = height("row-COMFORTABLE")
        val compact = height("row-COMPACT")
        assertTrue("compact $compact < comfortable $comfortable", compact < comfortable * 0.8f)
    }

    @Test
    fun tappingThePictureSelects() {
        var taps = 0
        show(MailboxDensity.COMFORTABLE, onAvatarClick = { taps++ })
        // The picture sits after the row's 8 dp padding and the unread dot's slot, 10 dp from the top.
        compose.onNodeWithTag("row-0").performTouchInput { click(Offset(40.dp.toPx(), 30.dp.toPx())) }
        assertEquals(1, taps)
        // A tap on the text opens instead (the row's own click), not the picture's.
        compose.onNodeWithTag("row-0").performTouchInput { click(Offset(width - 40.dp.toPx(), 30.dp.toPx())) }
        assertEquals(1, taps)
    }

    @Test
    fun aSelectedRowShowsACheckMarkInPlaceOfThePicture() {
        show(MailboxDensity.COMFORTABLE, selected = true)
        compose.onAllNodes(leading(RowLeading.CHECK_MARK)).assertCountEquals(1)
        compose.onAllNodes(leading(RowLeading.PICTURE)).assertCountEquals(0)
    }

    @Test
    fun thePictureIsDecorativeForTalkBack() {
        show(MailboxDensity.COMFORTABLE)
        // Still one item that names the sender: the picture adds nothing for TalkBack to read.
        compose
            .onNodeWithTag("row-0")
            .assert(
                SemanticsMatcher("reads the sender first") { node ->
                    node.config
                        .getOrElse(SemanticsProperties.ContentDescription) { emptyList() }
                        .singleOrNull()
                        ?.startsWith("From Ana López.") == true
                },
            )
        compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.ContentDescription)).assertCountEquals(1)
    }

    @Test
    fun theUnifiedInboxAccountColourStaysVisibleOnThePicture() {
        show(MailboxDensity.COMFORTABLE, accountId = 3)
        compose.onAllNodes(leading(RowLeading.PICTURE)).assertCountEquals(1)
    }

    private fun leading(kind: RowLeading) = SemanticsMatcher.expectValue(RowLeadingKey, kind)

    @Test
    fun compactSelectingShowsCheckboxes() {
        show(MailboxDensity.COMPACT, selected = true, rows = 2)
        compose.onAllNodes(leading(RowLeading.CHECKBOX)).assertCountEquals(2)
    }

    private fun height(tag: String): Dp =
        compose
            .onNodeWithTag(tag)
            .getBoundsInRoot()
            .height
}
