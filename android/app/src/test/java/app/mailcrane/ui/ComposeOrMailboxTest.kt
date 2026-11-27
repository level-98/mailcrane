package app.mailcrane.ui

import android.app.Application
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import app.mailcrane.ui.theme.MailcraneTheme
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Opening and closing Compose keeps where the mailbox was (#496). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class ComposeOrMailboxTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun closingComposeKeepsTheMailboxScrollPosition() {
        var composing by mutableStateOf(false)
        lateinit var list: LazyListState
        compose.setContent {
            MailcraneTheme {
                ComposeOrMailbox(
                    composing = composing,
                    compose = { shared -> Box(shared.fillMaxSize()) { Text("Compose") } },
                    mailbox = { shared ->
                        // A new list each time the mailbox comes back; its position must come back too.
                        list = rememberLazyListState()
                        LazyColumn(state = list, modifier = shared.fillMaxSize()) {
                            items(100) { Text("Row $it", Modifier.height(48.dp)) }
                        }
                    },
                )
            }
        }
        compose.runOnIdle { runBlocking { list.scrollToItem(60) } }
        compose.runOnIdle { composing = true }
        compose.onNodeWithText("Compose").assertIsDisplayed()
        compose.runOnIdle { composing = false }
        compose.waitForIdle()
        compose.runOnIdle { assertEquals(60, list.firstVisibleItemIndex) }
        compose.onNodeWithText("Row 60").assertIsDisplayed()
    }
}
