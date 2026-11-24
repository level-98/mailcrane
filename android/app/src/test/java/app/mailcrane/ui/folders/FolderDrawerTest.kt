package app.mailcrane.ui.folders

import android.app.Application
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
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
}
