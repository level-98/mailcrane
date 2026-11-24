package app.mailcrane.ui.components

import android.app.Application
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class EmptyStateTest {
    @get:Rule
    val compose = createComposeRule()

    private val hasContentDescription = SemanticsMatcher.keyIsDefined(SemanticsProperties.ContentDescription)

    @Test
    fun emptyFolderShowsTitleAndBody() {
        compose.setContent { MailcraneTheme { EmptyFolderState() } }
        compose.onNodeWithText("All caught up").assertIsDisplayed()
        compose.onNodeWithText("No messages in this folder.").assertIsDisplayed()
    }

    @Test
    fun noResultsShowsTitleAndBody() {
        compose.setContent { MailcraneTheme { NoResultsState() } }
        compose.onNodeWithText("No results").assertIsDisplayed()
        compose.onNodeWithText("Try different words or check your spelling.").assertIsDisplayed()
    }

    @Test
    fun welcomeButtonAddsAccount() {
        var clicked = false
        compose.setContent { MailcraneTheme { WelcomeState(onAddAccount = { clicked = true }) } }
        compose.onNodeWithText("Welcome to Mailcrane").assertIsDisplayed()
        compose.onNodeWithText("Add an email account to get started.").assertIsDisplayed()
        compose.onNodeWithText("Add account").performClick()
        assertTrue(clicked)
    }

    @Test
    fun illustrationIsHiddenFromScreenReaders() {
        compose.setContent { MailcraneTheme { WelcomeState(onAddAccount = {}) } }
        // Only text is read: no node carries a content description.
        compose.onAllNodes(hasContentDescription, useUnmergedTree = true).assertCountEquals(0)
    }

    @Test
    fun titleIsAHeading() {
        compose.setContent { MailcraneTheme { EmptyFolderState() } }
        compose
            .onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
            .assertCountEquals(1)
    }
}
