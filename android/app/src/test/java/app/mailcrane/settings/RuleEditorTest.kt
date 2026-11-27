package app.mailcrane.settings

import android.app.Application
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class RuleEditorTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun aRefusalIsShownInTheEditor() {
        // #482 review: a save the core refuses opens the editor again; its message must show there, not on the
        // list underneath.
        val snackbar = SnackbarHostState()
        compose.setContent {
            MailcraneTheme {
                RuleEditor(
                    draft = RuleDraft(name = "Old"),
                    folders = emptyList(),
                    onChange = {},
                    onSave = {},
                    onCancel = {},
                    snackbar = snackbar,
                )
                LaunchedEffect(Unit) { snackbar.showSnackbar("Pick another folder") }
            }
        }
        compose.onNodeWithText("Pick another folder").assertIsDisplayed()
    }
}
