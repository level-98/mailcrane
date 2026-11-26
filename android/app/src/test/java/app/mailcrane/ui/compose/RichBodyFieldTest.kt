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
 * The rich body field against a view model that is slower than the keyboard (#419): [state] is what the view
 * model last published, and the test decides when each edit comes back and when an outside change lands.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class RichBodyFieldTest {
    @get:Rule
    val compose = createComposeRule()

    private var state by mutableStateOf(ComposeUiState())

    /** What the field sent: its text and the body revision it was based on. */
    private val sent = mutableListOf<Pair<String, Long>>()

    /** The view model's state right now, when a test makes it run ahead of the composed [state]. */
    private var live: ComposeUiState? = null

    private fun show(actions: RichTextActions = RichTextActions()) =
        compose.setContent {
            MailcraneTheme {
                // The screen lays the toolbar, the chip and the field out in a column too.
                Column {
                    RichBodyField(
                        state = state,
                        onBodyChange = { text, revision -> sent += text to revision },
                        actions = actions,
                        enabled = true,
                        colors = TextFieldDefaults.colors(),
                        currentState = { live ?: state },
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

    /** The view model echoes one of the field's own edits: same revision. */
    private fun echo(body: String) {
        state = state.copy(body = body)
        compose.waitForIdle()
    }

    /** An outside change (reply, draft, signature, list, reset): a new revision. */
    private fun outside(
        body: String,
        cursorAtEnd: Boolean = false,
    ) {
        state = state.copy(body = body, bodyRevision = state.bodyRevision + 1, bodyCursorAtEnd = cursorAtEnd)
        compose.waitForIdle()
    }

    @Test
    fun ownEditsComingBackLateNeitherMoveTheCursorNorDropText() {
        show()
        field().performTextInput("Q")
        field().performTextInput("A")
        assertEquals(listOf("Q" to 0L, "QA" to 0L), sent)
        // The view model is one edit behind: "Q" comes back while the field already holds "QA".
        echo("Q")
        assertEquals("QA", text())
        assertEquals(TextRange(2), selection())
        echo("QA")
        field().performTextInput("B")
        assertEquals("QAB", text())
        assertEquals(TextRange(3), selection())
        assertEquals("QAB" to 0L, sent.last())
    }

    @Test
    fun manyEditsInFlightAllSurvive() {
        show()
        "loopback".forEach { field().performTextInput(it.toString()) }
        // The echoes trickle in, oldest first, long after the typing.
        sent.toList().forEach { (body, _) -> echo(body) }
        assertEquals("loopback", text())
        assertEquals(TextRange(8), selection())
    }

    @Test
    fun deletingBackToTheLastPublishedTextIsStillSent() {
        show()
        field().performTextInput("A")
        // Deleted before "A" came back: the view model still says "", but it is about to say "A".
        field().performTextClearance()
        assertEquals(listOf("A" to 0L, "" to 0L), sent.toList())
        echo("A")
        echo("")
        assertEquals("", text())
    }

    @Test
    fun anOutsideChangeWithATextTypedEarlierIsStillApplied() {
        // Review of #426: "a", backspace, "a" sent ["a", "", "a"]; a later outside change to "" (a reset) must
        // still land, though the field once sent "" itself.
        show()
        field().performTextInput("a")
        field().performTextClearance()
        field().performTextInput("a")
        echo("a")
        outside("")
        assertEquals("", text())
        field().performTextInput("b")
        assertEquals("b" to 1L, sent.last())
    }

    @Test
    fun editsAfterAnOutsideChangeCarryTheNewRevision() {
        // The view model rebases an edit typed against an older revision; the field says which one it used.
        show()
        field().performTextInput("H")
        outside("H\n\n-- \nAnn")
        field().performTextInput("i")
        assertEquals("H" to 0L, sent.first())
        assertEquals(1L, sent.last().second)
    }

    @Test
    fun aRebasedKeystrokeBringsItsCursor() {
        // Review of #426: the field showed "H"; the view model merged a late "i" into "• Hi" and says where the
        // cursor goes, so the next key lands after "Hi", not before it.
        show()
        field().performTextInput("H")
        state = state.copy(body = "• Hi", bodyRevision = 2, bodyCursor = 4)
        compose.waitForIdle()
        assertEquals("• Hi", text())
        assertEquals(TextRange(4), selection())
        field().performTextInput("!")
        assertEquals("• Hi!", text())
    }

    @Test
    fun onlyTheNewestRevisionIsApplied() {
        // Review of #426: the view model is already at revision 3 (a late "!" merged in) when the effect for
        // revision 2 runs; applying 2's text would drop the "!".
        show()
        live = state.copy(body = "Hi!\n\n-- \nAnn", bodyRevision = 3, bodyCursor = 3)
        state = state.copy(body = "Hi\n\n-- \nAnn", bodyRevision = 2)
        compose.waitForIdle()
        assertEquals("Hi!\n\n-- \nAnn", text())
        assertEquals(TextRange(3), selection())
        // Revision 3 reaching the composition later changes nothing.
        state = checkNotNull(live)
        live = null
        compose.waitForIdle()
        assertEquals("Hi!\n\n-- \nAnn", text())
        assertEquals(TextRange(3), selection())
    }

    @Test
    fun aReplyOrSignatureFromOutsideStillAppears() {
        show()
        outside("\n\n-- \nAnn")
        assertEquals("\n\n-- \nAnn", text())
        // A new message or a reply starts at the top, above the signature or quote.
        assertEquals(TextRange(0), selection())
    }

    @Test
    fun aLoadedDraftPutsTheCursorAtTheEnd() {
        show()
        outside("Saved draft", cursorAtEnd = true)
        assertEquals("Saved draft", text())
        assertEquals(TextRange(11), selection())
    }

    @Test
    fun aListToggleKeepsTheSelectionItAsksFor() {
        show(
            RichTextActions(
                onList = { _, _, _ ->
                    // The view model makes the line a bullet and says where the selection goes.
                    state = state.copy(body = "• one", bodyRevision = state.bodyRevision + 1)
                    2 to 5
                },
            ),
        )
        field().performTextInput("one")
        sent.toList().forEach { (body, _) -> echo(body) }
        compose.onNodeWithContentDescription("Bulleted list").performClick()
        compose.waitForIdle()
        assertEquals("• one", text())
        assertEquals(TextRange(2, 5), selection())
    }

    @Test
    fun aListToggleThatChangesNothingStillMovesTheSelection() {
        // Review of #426: a toggle leaving the text as it was still gets its selection, and leaves none behind.
        show(
            RichTextActions(
                onList = { _, _, _ ->
                    state = state.copy(bodyRevision = state.bodyRevision + 1)
                    0 to 3
                },
            ),
        )
        field().performTextInput("one")
        sent.toList().forEach { (body, _) -> echo(body) }
        compose.onNodeWithContentDescription("Bulleted list").performClick()
        compose.waitForIdle()
        assertEquals(TextRange(0, 3), selection())
        // A later outside change keeps the field's own selection, not a stale toggle range.
        outside("one two")
        assertEquals(TextRange(0, 3), selection())
    }
}
