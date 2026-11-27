package app.mailcrane.ui.search

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** After a chip rewrites the query, typing continues at the end of the new text (M4-21 review). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class SearchFieldCursorTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun keepsTheCursorWhileTheTextIsUnchanged() {
        val current = TextFieldValue("lunch has:attachment", TextRange(3))
        assertSame(current, fieldValueFor(current, "lunch has:attachment"))
    }

    @Test
    fun movesTheCursorToTheEndWhenTheQueryChangesElsewhere() {
        val current = TextFieldValue("lunch has:attachment", TextRange(5))
        val shown = fieldValueFor(current, "lunch has:attachment from:")
        assertEquals("lunch has:attachment from:", shown.text)
        assertEquals(TextRange(26), shown.selection)
    }

    @Test
    fun typingAfterTheFromChipLandsAfterFrom() {
        var query by mutableStateOf("")
        compose.setContent {
            MailcraneTheme {
                SearchContent(
                    state = SearchState(query = query),
                    recent = emptyList(),
                    onQueryChange = { query = it },
                    onSubmit = {},
                    onRecent = {},
                    onClearRecent = {},
                    onOpenMessage = {},
                    onBack = {},
                )
            }
        }
        val field = compose.onNode(hasSetTextAction())
        field.performTextInput("lunch")
        // What tapping the From chip does to the query.
        compose.runOnIdle { query = "lunch has:attachment from:" }
        field.performTextInput("Ana")
        compose.runOnIdle { assertEquals("lunch has:attachment from:Ana", query) }
    }

    /** #397: opened again with the last query still there, typing replaces it instead of adding to it. */
    @Test
    fun reopenedWithAnOldQueryTypingReplacesIt() {
        var query by mutableStateOf("test")
        compose.setContent {
            MailcraneTheme {
                SearchContent(
                    state = SearchState(query = query),
                    recent = emptyList(),
                    onQueryChange = { query = it },
                    onSubmit = {},
                    onRecent = {},
                    onClearRecent = {},
                    onOpenMessage = {},
                    onBack = {},
                )
            }
        }
        compose.onNode(hasSetTextAction()).performTextInput("Re Test")
        compose.runOnIdle { assertEquals("Re Test", query) }
    }
}
