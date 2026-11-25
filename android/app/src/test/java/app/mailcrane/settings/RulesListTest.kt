package app.mailcrane.settings

import android.app.Application
import androidx.compose.material3.SnackbarHostState
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import app.mailcrane.core.RuleActionInfo
import app.mailcrane.core.RuleConditionInfo
import app.mailcrane.core.RuleInfo
import app.mailcrane.core.RuleInput
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class RulesListTest {
    @get:Rule
    val compose = createComposeRule()

    private fun rule(
        id: Long,
        missingFolder: String?,
    ) = RuleInfo(
        id = id,
        rule =
            RuleInput(
                accountId = 1,
                name = "Rule $id",
                enabled = missingFolder == null,
                matchAll = true,
                stopProcessing = false,
                conditions = listOf(RuleConditionInfo.Subject("x")),
                actions = listOf(RuleActionInfo.Flag),
            ),
        missingFolder = missingFolder,
    )

    @Test
    fun aRuleTurnedOffForARemovedFolderSaysWhy() {
        compose.setContent {
            MailcraneTheme {
                RulesList(
                    state = RulesState(rules = listOf(rule(1, "Receipts"), rule(2, null)), loading = false),
                    snackbar = SnackbarHostState(),
                    onBack = {},
                    onAdd = {},
                    onOpen = {},
                    onEnabled = { _, _ -> },
                    onMove = { _, _ -> },
                )
            }
        }
        compose.onNodeWithTag("rule-missing-folder-1", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithText("Receipts", substring = true, useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("rule-missing-folder-2", useUnmergedTree = true).assertDoesNotExist()
    }
}
