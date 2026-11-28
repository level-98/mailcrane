package app.mailcrane.ui.conversation

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.messagelist.UndoOffer
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Criterion 3 of #571: deleting a conversation's last message closes it, and its Undo goes to the list. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class ConversationUndoHandOffTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun deletingTheLastMessageClosesAndHandsItsUndoToTheList() {
        val core =
            FakeThreadCore().apply {
                thread = listOf(threadMessage(1))
                deleteRemoves = true
            }
        val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
        var closed = false
        var handed: UndoOffer? = null
        compose.setContent {
            MailcraneTheme {
                ConversationScreen(
                    threadId = THREAD,
                    title = "Lunch",
                    onBack = { closed = true },
                    onOpenMessage = {},
                    onUndoHandOff = { handed = it },
                    viewModel = vm,
                )
            }
        }
        compose.waitForIdle()
        compose.runOnIdle { vm.swipe(threadMessage(1), SwipeAction.DELETE) }
        compose.waitForIdle()
        assertTrue("the empty conversation closed", closed)
        assertEquals("its Undo went to the list", UndoOffer(1, SwipeAction.DELETE), handed)
        assertNull("and isn't left for the next opening", vm.undo.value.offer)
    }

    /** #572 review: leaving by Back while an Undo is on offer hands it to the list too. */
    @Test
    fun leavingByBackHandsTheUndoToTheList() {
        val core = FakeThreadCore().apply { deleteRemoves = true }
        val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
        var shown by mutableStateOf(true)
        var handed: UndoOffer? = null
        compose.setContent {
            MailcraneTheme {
                if (shown) {
                    ConversationScreen(
                        threadId = THREAD,
                        title = "Lunch",
                        onBack = {},
                        onOpenMessage = {},
                        onUndoHandOff = { handed = it },
                        viewModel = vm,
                    )
                }
            }
        }
        compose.waitForIdle()
        compose.runOnIdle { vm.swipe(threadMessage(1), SwipeAction.DELETE) }
        compose.waitForIdle()
        assertNull("still open (message 2 is left), so not handed yet", handed)
        shown = false
        compose.waitForIdle()
        assertEquals(UndoOffer(1, SwipeAction.DELETE), handed)
        assertNull("not left for the next opening", vm.undo.value.offer)
    }
}
