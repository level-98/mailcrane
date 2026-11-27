package app.mailcrane.ui.components

import android.app.Application
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
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

    private fun showNoResults(
        height: Dp,
        fontScale: Float = 1f,
    ) {
        compose.setContent {
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, fontScale)) {
                MailcraneTheme { Box(Modifier.height(height)) { NoResultsState() } }
            }
        }
    }

    /** #397: with room for both, the picture shows above the text. */
    @Test
    fun withRoomTheIllustrationShows() {
        showNoResults(height = 600.dp)
        compose.onNodeWithTag(ILLUSTRATION_TAG).assertExists()
        compose.onNodeWithText("No results").assertIsDisplayed()
    }

    /** #397: above the keyboard, short of room, the picture goes and the text stays fully visible. */
    @Test
    fun shortOfRoomTheIllustrationGoesAndTheTextStays() {
        showNoResults(height = 300.dp)
        compose.onNodeWithTag(ILLUSTRATION_TAG).assertDoesNotExist()
        compose.onNodeWithText("No results").assertIsDisplayed()
        compose.onNodeWithText("Try different words or check your spelling.").assertIsDisplayed()
    }

    /** #397: in a height that holds both at normal text size... */
    @Test
    fun aMidHeightHoldsTheIllustrationAtNormalText() {
        showNoResults(height = 360.dp)
        compose.onNodeWithTag(ILLUSTRATION_TAG).assertExists()
    }

    /** ...large text needs more room for the text, so the picture goes first. */
    @Test
    fun largeTextGivesUpTheIllustrationFirst() {
        showNoResults(height = 360.dp, fontScale = 2f)
        compose.onNodeWithTag(ILLUSTRATION_TAG).assertDoesNotExist()
        compose.onNodeWithText("No results").assertIsDisplayed()
    }

    @Test
    fun titleIsAHeading() {
        compose.setContent { MailcraneTheme { EmptyFolderState() } }
        compose
            .onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
            .assertCountEquals(1)
    }
}
