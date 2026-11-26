package app.mailcrane.ui.compose

import android.app.Application
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.text.TextRange
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The rich body field against a view model that is slower than the keyboard (#419): [state] is
 * what the view model last published, and the test decides when each edit comes back.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class RichBodyFieldTest {
    @get:Rule
    val compose = createComposeRule()

    private var state by mutableStateOf(ComposeUiState())
    private val sent = mutableListOf<String>()

    private fun show(actions: RichTextActions = RichTextActions()) =
        compose.setContent {
            MailcraneTheme {
                // The screen lays the toolbar, the chip and the field out in a column too.
                Column {
                    RichBodyField(
                        state = state,
                        onBodyChange = { sent += it },
                        actions = actions,
                        enabled = true,
                        colors = TextFieldDefaults.colors(),
                    )
                }
            }
        }

    private fun field() = compose.onNodeWithTag("compose-body")

    private fun text() =
        field()
            .fetchSemanticsNode()
            .config[SemanticsProperties.EditableText]
            .text

    private fun selection() = field().fetchSemanticsNode().config[SemanticsProperties.TextSelectionRange]

    /** The view model publishes [body], as if an edit (or an outside change) just reached it. */
    private fun publish(
        body: String,
        cursorAtEnd: Boolean = state.bodyCursorAtEnd,
    ) {
        state = state.copy(body = body, bodyCursorAtEnd = cursorAtEnd)
        compose.waitForIdle()
    }

    @Test
    fun ownEditsComingBackLateNeitherMoveTheCursorNorDropText() {
        show()
        field().performTextInput("Q")
        field().performTextInput("A")
        assertEquals(listOf("Q", "QA"), sent)
        // The view model is one edit behind: "Q" comes back while the field already holds "QA".
        publish("Q")
        assertEquals("QA", text())
        assertEquals(TextRange(2), selection())
        publish("QA")
        field().performTextInput("B")
        assertEquals("QAB", text())
        assertEquals(TextRange(3), selection())
        assertEquals("QAB", sent.last())
    }

    @Test
    fun manyEditsInFlightAllSurvive() {
        show()
        "loopback".forEach { field().performTextInput(it.toString()) }
        // The echoes trickle in, oldest first, long after the typing.
        sent.toList().forEach { publish(it) }
        assertEquals("loopback", text())
        assertEquals(TextRange(8), selection())
    }

    @Test
    fun deletingBackToTheLastPublishedTextIsStillSent() {
        show()
        field().performTextInput("A")
        // Deleted before "A" came back: the view model still says "", but it is about to say "A".
        field().performTextClearance()
        assertEquals(listOf("A", ""), sent)
        publish("A")
        publish("")
        assertEquals("", text())
    }

    @Test
    fun aReplyOrSignatureFromOutsideStillAppears() {
        show()
        publish("\n\n-- \nAnn")
        assertEquals("\n\n-- \nAnn", text())
        // A new message or a reply starts at the top, above the signature or quote.
        assertEquals(TextRange(0), selection())
    }

    @Test
    fun aLoadedDraftPutsTheCursorAtTheEnd() {
        show()
        publish("Saved draft", cursorAtEnd = true)
        assertEquals("Saved draft", text())
        assertEquals(TextRange(11), selection())
    }

    @Test
    fun aListToggleKeepsTheSelectionItAsksFor() {
        show(
            RichTextActions(
                onList = { _, _, _ ->
                    // The view model makes the line a bullet and says where the selection goes.
                    state = state.copy(body = "• one")
                    2 to 5
                },
            ),
        )
        field().performTextInput("one")
        sent.toList().forEach { publish(it) }
        compose.onNodeWithContentDescription("Bulleted list").performClick()
        compose.waitForIdle()
        assertEquals("• one", text())
        assertEquals(TextRange(2, 5), selection())
    }
}
