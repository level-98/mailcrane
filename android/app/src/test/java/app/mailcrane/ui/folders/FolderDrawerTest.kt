package app.mailcrane.ui.folders

import android.app.Application
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class FolderDrawerTest {
    @get:Rule
    val compose = createComposeRule()

    private val folders =
        listOf(
            FolderSummary(id = 1, displayName = "INBOX", role = FolderRole.INBOX, depth = 0u, unreadCount = 3u),
            FolderSummary(id = 2, displayName = "Sent Items", role = FolderRole.SENT, depth = 0u, unreadCount = 0u),
            FolderSummary(id = 3, displayName = "Work", role = null, depth = 0u, unreadCount = 1u),
            FolderSummary(id = 4, displayName = "Projects", role = null, depth = 1u, unreadCount = 1_500u),
        )

    private fun show(onSelect: (Long) -> Unit = {}) {
        compose.setContent {
            MailcraneTheme { FolderDrawer(folders = folders, selectedFolderId = 1, onSelect = onSelect) }
        }
    }

    @Test
    fun standardFoldersShowTranslatedNamesAndOthersTheirServerName() {
        show()
        compose.onNodeWithContentDescription("Inbox", substring = true).assertExists()
        compose.onNodeWithContentDescription("Sent").assertExists()
        compose.onNodeWithContentDescription("INBOX", substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Sent Items").assertDoesNotExist()
        compose.onNodeWithContentDescription("Work", substring = true).assertExists()
    }

    @Test
    fun gmailsStarredAndImportantShowTheirNamesAndIcons() {
        // #390: they showed "[Gmail]/Starred" and "[Gmail]/Important" with the generic folder icon.
        val gmail =
            listOf(
                FolderSummary(id = 1, displayName = "INBOX", role = FolderRole.INBOX, depth = 0u, unreadCount = 0u),
                FolderSummary(
                    id = 7,
                    displayName = "[Gmail]/Starred",
                    role = FolderRole.STARRED,
                    depth = 0u,
                    unreadCount = 0u,
                ),
                FolderSummary(
                    id = 8,
                    displayName = "[Gmail]/Important",
                    role = FolderRole.IMPORTANT,
                    depth = 0u,
                    unreadCount = 2u,
                ),
            )
        compose.setContent { MailcraneTheme { FolderDrawer(folders = gmail, selectedFolderId = 1, onSelect = {}) } }
        compose.onNodeWithContentDescription("Starred", substring = true).assertExists()
        compose.onNodeWithContentDescription("Important", substring = true).assertExists()
        compose.onNodeWithContentDescription("[Gmail]", substring = true).assertDoesNotExist()
        assertEquals(R.drawable.ic_mc_star, folderIcon(FolderRole.STARRED))
        assertEquals(R.drawable.ic_mc_important, folderIcon(FolderRole.IMPORTANT))
        assertEquals(R.string.folder_starred, roleName(FolderRole.STARRED))
        assertEquals(R.string.folder_important, roleName(FolderRole.IMPORTANT))
    }

    @Test
    fun virtualFoldersAreNeverMoveTargets() {
        // #390: a message is in Starred or Important because of a flag or Gmail's judgement, not a move.
        val starred = FolderSummary(id = 7, displayName = "x", role = FolderRole.STARRED, depth = 0u, unreadCount = 0u)
        val important = starred.copy(id = 8, role = FolderRole.IMPORTANT)
        assertEquals(false, starred.takesMoves)
        assertEquals(false, important.takesMoves)
        assertEquals(true, folders.all { it.takesMoves })
    }

    @Test
    fun talkBackReadsTheNameAndUnreadCountOnce() {
        show()
        compose.onNodeWithContentDescription("Inbox, 3 unread").assertIsSelected()
        compose.onNodeWithContentDescription("Sent").assertExists()
        compose.onNodeWithContentDescription("Work, 1 unread").assertExists()
        // The full count is read even though the badge shows "999+".
        compose.onNodeWithContentDescription("Projects, 1500 unread").assertExists()
        // The visible label and badge are silent, so nothing is read twice.
        compose.onNodeWithText("Inbox", substring = true, useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("999+", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun nestedFoldersAreIndented() {
        show()
        val top = compose.onNodeWithContentDescription("Work, 1 unread").fetchSemanticsNode().boundsInRoot
        val nested = compose.onNodeWithContentDescription("Projects, 1500 unread").fetchSemanticsNode().boundsInRoot
        assertEquals(16f * compose.density.density, nested.left - top.left, 0.5f)
    }

    @Test
    fun choosingAFolderReportsIt() {
        var chosen: Long? = null
        show { chosen = it }
        compose.onNodeWithContentDescription("Work, 1 unread").performClick()
        assertEquals(3L, chosen)
    }

    /**
     * #336: the permanent pane beside the list (landscape, expanded width) with several accounts and
     * every action, in a landscape phone's height. Inbox shows under "Folders", and the rows after
     * the folders are a scroll away rather than squeezing the folders out.
     */
    @Test
    fun permanentPaneListsFoldersInALandscapeHeight() {
        val accounts = (1L..3L).map { AccountSummary(it, "user$it@example.org", "User $it") }
        compose.setContent {
            MailcraneTheme {
                // A Medium Phone in landscape: 915 x 411 dp, less the status bar.
                Row(Modifier.size(915.dp, 380.dp)) {
                    FolderDrawer(
                        folders = folders,
                        selectedFolderId = 1,
                        onSelect = {},
                        onOpenSettings = {},
                        onOpenLocalDrafts = {},
                        outboxCount = 2,
                        onOpenOutbox = {},
                        onManageFolders = {},
                        accounts = accounts,
                        currentAccountId = 1,
                        permanent = true,
                    )
                }
            }
        }
        val header = compose.onNodeWithText("Folders").fetchSemanticsNode().boundsInRoot
        val inbox = compose.onNodeWithContentDescription("Inbox, 3 unread")
        inbox.assertIsDisplayed().assertIsSelected()
        val inboxBounds = inbox.fetchSemanticsNode().boundsInRoot
        assert(inboxBounds.top >= header.bottom) { "Inbox is under the Folders header" }
        assert(inboxBounds.height > 0f) { "Inbox has room" }
        compose.onNodeWithTag("drawer-list").performScrollToNode(hasText("Settings"))
        compose.onNodeWithText("Settings").assertIsDisplayed()
    }
}
