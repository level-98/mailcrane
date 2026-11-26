package app.mailcrane.ui.account

import android.app.Application
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * #343: after adding an account, "Add account" must open the setup screen again. MainActivity shows
 * [ManualSetupScreen] while it's adding an account and keeps one view model for the activity's life;
 * this does the same with one shared view model.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class AddAccountAgainTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun addAccountOpensSetupAgainAfterAnAdd() {
        var nextId = 7L
        val vm = ManualSetupViewModel(FakeSetupCore { nextId++ })
        var adding by mutableStateOf(true)
        val added = mutableListOf<Long>()
        compose.setContent {
            MailcraneTheme {
                if (adding) {
                    ManualSetupScreen(
                        onAccountAdded = {
                            adding = false
                            added += it
                        },
                        viewModel = vm,
                    )
                } else {
                    TextButton(onClick = { adding = true }) { Text("Add another account") }
                }
            }
        }

        // The first account is added and the app goes back to the inbox.
        compose.runOnIdle {
            vm.onEmailChange("one@example.org")
            vm.submit()
        }
        compose.waitForIdle()
        assertEquals(listOf(7L), added)
        assertTrue(!adding)

        // "Add account" again: the setup screen stays open, empty, and nothing is reported twice.
        compose.onNodeWithText("Add another account").performClick()
        compose.waitForIdle()
        assertTrue("setup screen stays open", adding)
        assertEquals(listOf(7L), added)
        assertEquals("", vm.state.value.email)

        // And a second account can be added.
        compose.runOnIdle {
            vm.onEmailChange("two@example.org")
            vm.submit()
        }
        compose.waitForIdle()
        assertEquals(listOf(7L, 8L), added)
    }
}
