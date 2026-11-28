package app.mailcrane.settings

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import app.mailcrane.ui.messagelist.swipeIconTag
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Settings' sample row for choosing the swipe actions (#553). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class SwipePreviewSectionTest {
    @get:Rule
    val compose = createComposeRule()

    private var right by mutableStateOf(SwipeAction.ARCHIVE)
    private var left by mutableStateOf(SwipeAction.DELETE)

    private fun show() {
        compose.setContent {
            MailcraneTheme {
                SwipePreviewSection(
                    SwipeSettings(
                        right = right,
                        left = left,
                        onRightChange = { right = it },
                        onLeftChange = { left = it },
                    ),
                )
            }
        }
    }

    /** The sample row's TalkBack actions, by label. */
    private fun actionLabels(): List<String> =
        compose
            .onNode(SemanticsMatcher.keyIsDefined(SemanticsActions.CustomActions))
            .fetchSemanticsNode()
            .config[SemanticsActions.CustomActions]
            .map { it.label }

    @Test
    fun swipingRightOpensThatDirectionsPickerAndPickingSavesIt() {
        show()
        compose.onNodeWithTag(SWIPE_PREVIEW_TAG).performTouchInput { swipeRight() }
        compose.waitForIdle()
        compose.onNodeWithText("Flag or unflag").performClick()
        compose.waitForIdle()
        assertEquals(SwipeAction.FLAG, right)
        assertEquals("left is untouched", SwipeAction.DELETE, left)
        // The sample shows it at once: its TalkBack action names the new choice.
        assertEquals(listOf("Swipe right: Flag or unflag", "Swipe left: Delete"), actionLabels())
    }

    @Test
    fun swipingLeftOpensTheLeftPicker() {
        show()
        compose.onNodeWithTag(SWIPE_PREVIEW_TAG).performTouchInput { swipeLeft() }
        compose.waitForIdle()
        compose.onNodeWithText("Mark read or unread").performClick()
        compose.waitForIdle()
        assertEquals(SwipeAction.READ_UNREAD, left)
        assertEquals(SwipeAction.ARCHIVE, right)
    }

    /** Swiping the sample runs no mail action: closing the picker changes nothing at all. */
    @Test
    fun aSwipeAloneChangesNothing() {
        show()
        compose.onNodeWithTag(SWIPE_PREVIEW_TAG).performTouchInput { swipeRight() }
        compose.waitForIdle()
        compose.onNodeWithText("Cancel").performClick()
        compose.waitForIdle()
        assertEquals(SwipeAction.ARCHIVE, right)
        assertEquals(SwipeAction.DELETE, left)
    }

    /** TalkBack can choose both directions without swiping, even when they run the same action. */
    @Test
    fun talkBackHasOneActionPerDirection() {
        left = SwipeAction.ARCHIVE
        show()
        assertEquals(listOf("Swipe right: Archive", "Swipe left: Archive"), actionLabels())
    }

    /** A direction set to "none" can't be swiped on the sample, as in the list. */
    @Test
    fun aDirectionSetToNoneDoesNotSwipe() {
        left = SwipeAction.NONE
        show()
        compose.onNodeWithTag(SWIPE_PREVIEW_TAG).performTouchInput { swipeLeft() }
        compose.waitForIdle()
        compose.onNodeWithText("Cancel").assertDoesNotExist()
        assertEquals(listOf("Swipe right: Archive"), actionLabels())
    }

    /** The sample shows the chosen action's icon while it's dragged, as the list does (#594 review). */
    @Test
    fun theSampleShowsTheChosenActionsIcon() {
        right = SwipeAction.FLAG
        show()
        compose.onNodeWithTag(SWIPE_PREVIEW_TAG).performTouchInput {
            down(center)
            moveBy(Offset(width / 4f, 0f))
        }
        compose.waitForIdle()
        compose.onNodeWithTag(swipeIconTag(SwipeAction.FLAG), useUnmergedTree = true).assertExists()
        compose.onNodeWithTag(SWIPE_PREVIEW_TAG).performTouchInput { up() }
    }
}
