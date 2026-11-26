package app.mailcrane.ui.compose

import android.app.Application
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
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
 * The subject field against a view model that is slower than the keyboard (#429): [state] is what the view model
 * last published, and the test decides when each edit comes back and when an outside change lands.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class SubjectFieldTest {
    @get:Rule
    val compose = createComposeRule()

    private var state by mutableStateOf(ComposeUiState())

    /** What the view model holds right now, when a test makes it run ahead of the composed [state]. */
    private var live: ComposeUiState? = null

    /** What the field sent: its text and the subject revision it was based on. */
    private val sent = mutableListOf<Pair<String, Long>>()

    private fun show() =
        compose.setContent {
            MailcraneTheme {
                SubjectField(
                    state = state,
                    onSubjectChange = { text, revision -> sent += text to revision },
                    enabled = true,
                    colors = TextFieldDefaults.colors(),
                    currentState = { live ?: state },
                )
            }
        }

    private fun field() = compose.onNodeWithTag("compose-subject")

    private fun text() =
        field()
            .fetchSemanticsNode()
            .config[SemanticsProperties.EditableText]
            .text

    private fun selection() = field().fetchSemanticsNode().config[SemanticsProperties.TextSelectionRange]

    private fun echo(subject: String) {
        state = state.copy(subject = subject)
        compose.waitForIdle()
    }

    private fun outside(subject: String) {
        state = state.copy(subject = subject, subjectRevision = state.subjectRevision + 1)
        compose.waitForIdle()
    }

    @Test
    fun ownEditsComingBackLateNeitherMoveTheCursorNorDropText() {
        show()
        field().performTextInput("Q")
        field().performTextInput("u")
        // The view model is one edit behind: "Q" comes back while the field already holds "Qu".
        echo("Q")
        assertEquals("Qu", text())
        assertEquals(TextRange(2), selection())
        echo("Qu")
        field().performTextInput("a")
        assertEquals("Qua", text())
        assertEquals(listOf("Q" to 0L, "Qu" to 0L, "Qua" to 0L), sent)
    }

    @Test
    fun manyEditsInFlightAllSurvive() {
        show()
        "Quarterly".forEach { field().performTextInput(it.toString()) }
        sent.toList().forEach { (subject, _) -> echo(subject) }
        assertEquals("Quarterly", text())
        assertEquals(TextRange(9), selection())
    }

    @Test
    fun deletingBackToTheLastPublishedTextIsStillSent() {
        show()
        field().performTextInput("A")
        field().performTextClearance()
        assertEquals(listOf("A" to 0L, "" to 0L), sent.toList())
    }

    @Test
    fun aLoadedSubjectAppearsWithTheCursorAtItsEnd() {
        show()
        outside("Re: Lunch")
        assertEquals("Re: Lunch", text())
        assertEquals(TextRange(9), selection())
        field().performTextInput("!")
        assertEquals("Re: Lunch!" to 1L, sent.last())
    }

    @Test
    fun anOutsideChangeWithATextTypedEarlierIsStillApplied() {
        show()
        field().performTextInput("a")
        field().performTextClearance()
        field().performTextInput("a")
        echo("a")
        // A reset to "" lands though the field once sent "" itself.
        outside("")
        assertEquals("", text())
    }

    @Test
    fun aLoadThatReplacesTypedTextPutsTheCursorAtItsEnd() {
        // Review of #443: "ab" typed while a reply loads; the reply's subject replaces it, and the view model
        // answers the stale keystrokes by moving the revision on again. The next key goes after the subject, not
        // into the middle of it ("Rec: Lunch").
        show()
        field().performTextInput("a")
        field().performTextInput("b")
        assertEquals(TextRange(2), selection())
        outside("Re: Lunch")
        assertEquals("Re: Lunch", text())
        assertEquals(TextRange(9), selection())
        // The view model drops "ab" (typed against revision 0) and moves the revision on.
        outside("Re: Lunch")
        field().performTextInput("!")
        assertEquals("Re: Lunch!", text())
        assertEquals("Re: Lunch!" to state.subjectRevision, sent.last())
    }

    @Test
    fun onlyTheNewestRevisionIsApplied() {
        show()
        live = state.copy(subject = "Fwd: Plans", subjectRevision = 2)
        state = state.copy(subject = "Re: Plans", subjectRevision = 1)
        compose.waitForIdle()
        assertEquals("Fwd: Plans", text())
    }
}
