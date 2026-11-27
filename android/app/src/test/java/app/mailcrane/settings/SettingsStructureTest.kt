package app.mailcrane.settings

import android.app.Application
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import app.mailcrane.core.AccountSettingsCore
import app.mailcrane.core.AccountSignature
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.SaveSentSetting
import app.mailcrane.ui.folders.ManageFoldersContent
import app.mailcrane.ui.folders.ManageFoldersState
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Settings headings (#394) and each account's own settings page (#396). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB")
class SettingsStructureTest {
    @get:Rule
    val compose = createComposeRule()

    private val heading = SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)
    private val accountA = AccountSummary(1, "a.user@example.com", "Ann")
    private val accountB = AccountSummary(2, "b.user@example.com", "Ben")

    private fun showSettings() =
        compose.setContent {
            MailcraneTheme { SettingsContent(cleanLinks = true, onCleanLinksChange = {}, onBack = {}) }
        }

    @Test
    fun theLinkSwitchesHaveAHeading() {
        showSettings()
        compose.onNode(hasText("Links") and heading).assertIsDisplayed()
    }

    @Test
    fun theThemeChoicesAreOneLabelledGroup() {
        showSettings()
        compose.onNode(hasText("Theme") and heading).performScrollTo().assertIsDisplayed()
        val group =
            SemanticsMatcher.keyIsDefined(SemanticsProperties.SelectableGroup) and
                hasAnyDescendant(hasText("Theme")) and
                hasAnyDescendant(hasText("Follow the system")) and
                hasAnyDescendant(hasText("Light")) and
                hasAnyDescendant(hasText("Dark"))
        compose.onAllNodes(group).assertCountEquals(1)
    }

    @Test
    fun tappingAnAccountRowOpensThatAccount() {
        val opened = mutableListOf<Long>()
        compose.setContent {
            MailcraneTheme {
                AccountsSection(listOf(accountA, accountB), onAdd = {}, onRemove = {}, onOpen = {
                    opened +=
                        it
                })
            }
        }
        compose.onNodeWithTag("account-row-2").performClick()
        assertEquals(listOf(2L), opened)
    }

    @Test
    fun anAccountWithoutANameShowsItsAddressOnce() {
        val unnamed = AccountSummary(3, "c.user@example.com", "")
        compose.setContent {
            MailcraneTheme { AccountsSection(listOf(unnamed, accountA), onAdd = {}, onRemove = {}) }
        }
        // Unmerged: the clickable row merges its texts into one node (#492 review).
        compose.onAllNodes(hasText("c.user@example.com"), useUnmergedTree = true).assertCountEquals(1)
        // A named account still shows its address under the name.
        compose.onNodeWithText("Ann").assertIsDisplayed()
        compose.onNodeWithText("a.user@example.com").assertIsDisplayed()
        assertFalse(showsAddressApart(AccountSummary(4, "d@example.com", " D@example.com ")))
        assertTrue(showsAddressApart(accountB))
    }

    /**
     * #396: account B's page, opened whatever mailbox is current, names B and hands B's id to every
     * option on it, so an edit there (here a signature save) goes to B.
     */
    @Test
    fun accountBsPageNamesBAndEditsB() {
        val edits = mutableListOf<Long>()
        compose.setContent {
            MailcraneTheme {
                AccountSettingsPage(
                    account = accountB,
                    onBack = {},
                    onRemove = {},
                    sections = { id ->
                        Button(onClick = { edits += id }) { Text("Save signature") }
                    },
                )
            }
        }
        compose.onNodeWithText("Ben").assertIsDisplayed()
        compose.onNodeWithText("b.user@example.com").assertIsDisplayed()
        compose.onNodeWithText("Save signature").performClick()
        assertEquals(listOf(accountB.id), edits)
    }

    /** Remembers which account each call was for. */
    private class RecordingSettings : AccountSettingsCore {
        val signatureSavedFor = mutableListOf<Long>()

        override suspend fun getSignature(accountId: Long) = AccountSignature("", true)

        override suspend fun setSignature(
            accountId: Long,
            signature: AccountSignature,
        ) {
            signatureSavedFor += accountId
        }

        override suspend fun getSaveSent(accountId: Long) = SaveSentSetting.AUTOMATIC

        override suspend fun setSaveSent(
            accountId: Long,
            setting: SaveSentSetting,
        ) = Unit

        override suspend fun getAccountColor(accountId: Long): Int? = null

        override suspend fun setAccountColor(
            accountId: Long,
            color: Int?,
        ) = Unit

        override fun automaticAccountColor(accountId: Long) = 0
    }

    /**
     * #492 review: through the real signature section and its view model, a signature saved on
     * account B's page reaches the core for B (id 2), whatever mailbox is current.
     */
    @Test
    fun aSignatureSavedOnBsPageIsSavedForB() {
        val core = RecordingSettings()
        compose.setContent {
            MailcraneTheme {
                AccountSettingsPage(
                    account = accountB,
                    onBack = {},
                    onRemove = {},
                    sections = { id -> SignatureSection(id, viewModel = AccountSettingsViewModel(core, id)) },
                )
            }
        }
        compose.onNode(hasSetTextAction()).performTextInput("Ben\nExample Ltd")
        compose.onNodeWithText("Save signature").performClick()
        compose.runOnIdle { assertEquals(listOf(accountB.id), core.signatureSavedFor) }
    }

    /** #396: Manage folders names the account whose folders it shows. */
    @Test
    fun manageFoldersNamesItsAccount() {
        compose.setContent {
            MailcraneTheme {
                ManageFoldersContent(
                    state = ManageFoldersState(),
                    onBack = {},
                    onCreate = {},
                    onRename = {},
                    onMove = {},
                    onDelete = {},
                    onDismiss = {},
                    onConfirmCreate = {},
                    onConfirmRename = {},
                    onConfirmMove = {},
                    onConfirmDelete = {},
                    accountEmail = "b.user@example.com",
                )
            }
        }
        compose.onNode(heading and hasText("b.user@example.com", substring = true)).assertIsDisplayed()
    }

    @Test
    fun removingFromThePageAsksThenRemovesThatAccountAndGoesBack() {
        val removed = mutableListOf<Long>()
        var back = 0
        compose.setContent {
            MailcraneTheme {
                AccountSettingsPage(
                    account = accountB,
                    onBack = { back++ },
                    onRemove = { removed += it },
                    sections = {},
                )
            }
        }
        compose.onNodeWithText("Remove account").performClick()
        assertTrue(removed.isEmpty())
        compose.onNodeWithText("Remove").performClick()
        assertEquals(listOf(accountB.id), removed)
        assertEquals(1, back)
    }
}
