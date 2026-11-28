package app.mailcrane.settings

import android.app.Application
import androidx.compose.material3.SnackbarHostState
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
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
        actions: List<RuleActionInfo> = listOf(RuleActionInfo.Flag),
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
                actions = actions,
            ),
        missingFolder = missingFolder,
    )

    @Test
    fun aRuleTurnedOffForARemovedFolderSaysWhy() {
        compose.setContent {
            MailcraneTheme {
                RulesList(
                    state =
                        RulesState(
                            rules = listOf(rule(1, "Receipts", actions = emptyList()), rule(2, null)),
                            loading = false,
                        ),
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
        // With no actions left it can't be switched on; the other rule can be switched.
        compose.onNodeWithTag("rule-switch-1", useUnmergedTree = true).assertIsNotEnabled()
        compose.onNodeWithTag("rule-switch-2", useUnmergedTree = true).assertIsEnabled()
    }

    @Test
    fun eachRuleSwitchIsNamedAfterItsRule() {
        // #506: TalkBack read every switch as just "Switch, on".
        compose.setContent {
            MailcraneTheme {
                RulesList(
                    state = RulesState(rules = listOf(rule(1, null), rule(2, null)), loading = false),
                    snackbar = SnackbarHostState(),
                    onBack = {},
                    onAdd = {},
                    onOpen = {},
                    onEnabled = { _, _ -> },
                    onMove = { _, _ -> },
                )
            }
        }
        compose.onNodeWithTag("rule-switch-1", useUnmergedTree = true).assertContentDescriptionEquals("Rule 1")
        compose.onNodeWithTag("rule-switch-2", useUnmergedTree = true).assertContentDescriptionEquals("Rule 2")
    }

    private val inbox =
        FolderSummary(id = 10, displayName = "INBOX", role = FolderRole.INBOX, depth = 0u, unreadCount = 0u)
    private val starred =
        FolderSummary(
            id = 11,
            displayName = "Starred",
            role = FolderRole.STARRED,
            depth = 0u,
            unreadCount = 0u,
            canTakeMoves = false,
        )
    private val receipts = FolderSummary(id = 12, displayName = "Receipts", role = null, depth = 0u, unreadCount = 0u)

    @Test
    fun aRuleMovingIntoAFolderThatCantTakeMessagesSaysSo() {
        // #465: saved before #390. It isn't turned off (its mark-read still runs), but it says why it moves nothing.
        val old = rule(1, null, actions = listOf(RuleActionInfo.MarkRead, RuleActionInfo.Move(starred.id)))
        val fine = rule(2, null, actions = listOf(RuleActionInfo.Move(receipts.id)))
        compose.setContent {
            MailcraneTheme {
                RulesList(
                    state =
                        RulesState(
                            rules = listOf(old, fine),
                            folders = listOf(inbox, starred, receipts),
                            loading = false,
                        ),
                    snackbar = SnackbarHostState(),
                    onBack = {},
                    onAdd = {},
                    onOpen = {},
                    onEnabled = { _, _ -> },
                    onMove = { _, _ -> },
                )
            }
        }
        compose.onNodeWithTag("rule-virtual-target-1", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("rule-virtual-target-2", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("rule-switch-1", useUnmergedTree = true).assertIsEnabled()
    }
}
