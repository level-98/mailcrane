package app.mailcrane.ui.search

import android.app.Application
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.SoftwareKeyboardController
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertIsNotFocused
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.performImeAction
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

    private fun showWithQuery(
        initial: String,
        selectQuery: Boolean,
    ): () -> String {
        var query by mutableStateOf(initial)
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
                    selectQuery = selectQuery,
                )
            }
        }
        return { query }
    }

    /** Counts requests to hide the keyboard. */
    private class FakeKeyboard : SoftwareKeyboardController {
        var hides = 0

        override fun show() = Unit

        override fun hide() {
            hides++
        }
    }

    private fun showForSearchKey(keyboard: FakeKeyboard): Pair<() -> Int, () -> String> {
        var query by mutableStateOf("")
        var submitted = 0
        compose.setContent {
            CompositionLocalProvider(LocalSoftwareKeyboardController provides keyboard) {
                MailcraneTheme {
                    SearchContent(
                        state = SearchState(query = query),
                        recent = emptyList(),
                        onQueryChange = { query = it },
                        onSubmit = { submitted++ },
                        onRecent = {},
                        onClearRecent = {},
                        onOpenMessage = {},
                        onBack = {},
                    )
                }
            }
        }
        return { submitted } to { query }
    }

    /** #510: the keyboard's Search key runs the search and closes the keyboard, so the results show. */
    @Test
    fun theSearchKeySubmitsAndClosesTheKeyboard() {
        val keyboard = FakeKeyboard()
        val (submitted, _) = showForSearchKey(keyboard)
        val field = compose.onNode(hasSetTextAction())
        field.performTextInput("lunch")
        field.performImeAction()
        compose.runOnIdle {
            assertEquals(1, submitted())
            assertEquals(1, keyboard.hides)
        }
        field.assertIsNotFocused()
    }

    /** #510: with nothing typed, the Search key does nothing and the keyboard stays for typing. */
    @Test
    fun theSearchKeyWithAnEmptyQueryKeepsTheKeyboard() {
        val keyboard = FakeKeyboard()
        val (submitted, _) = showForSearchKey(keyboard)
        val field = compose.onNode(hasSetTextAction())
        field.performImeAction()
        compose.runOnIdle {
            assertEquals(0, submitted())
            assertEquals(0, keyboard.hides)
        }
        field.assertIsFocused()
    }

    /** #397: search opened again with the last query still there: typing replaces it, not adds to it. */
    @Test
    fun aFreshOpeningWithAnOldQueryTypingReplacesIt() {
        val query = showWithQuery("test", selectQuery = true)
        compose.onNode(hasSetTextAction()).performTextInput("Re Test")
        compose.runOnIdle { assertEquals("Re Test", query()) }
    }

    /** #481 review: back from a result in the same search, typing narrows the query, as before. */
    @Test
    fun backFromAResultTypingAddsToTheQuery() {
        val query = showWithQuery("lunch", selectQuery = false)
        compose.onNode(hasSetTextAction()).performTextInput(" from:ana")
        compose.runOnIdle { assertEquals("lunch from:ana", query()) }
    }
}
