package app.mailcrane.settings

import android.app.Application
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.emptyPreferences
import app.mailcrane.ui.theme.MailcraneTheme
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class ReadingSettingsTest {
    @get:Rule
    val compose = createComposeRule()

    /** A settings file kept in memory; a second [DataStoreAppSettings] on it is the app after a restart. */
    private class MemoryDataStore : DataStore<Preferences> {
        override val data = MutableStateFlow(emptyPreferences())

        override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences {
            val next = transform(data.value)
            data.value = next
            return next
        }
    }

    @Test
    fun defaultsKeepTodaysBehaviour() =
        runTest {
            val settings = DataStoreAppSettings(MemoryDataStore())
            assertEquals(MarkReadMode.ON_OPEN, settings.markReadMode.first())
            assertEquals(AfterMoveAction.LIST, settings.afterMoveAction.first())
        }

    @Test
    fun bothChoicesSurviveARestart() =
        runTest {
            val file = MemoryDataStore()
            DataStoreAppSettings(file).apply {
                setMarkReadMode(MarkReadMode.AFTER_DELAY)
                setAfterMoveAction(AfterMoveAction.PREVIOUS)
            }
            val restarted = DataStoreAppSettings(file)
            assertEquals(MarkReadMode.AFTER_DELAY, restarted.markReadMode.first())
            assertEquals(AfterMoveAction.PREVIOUS, restarted.afterMoveAction.first())
        }

    @Test
    fun theSectionShowsAndChangesBothChoices() {
        var markRead = MarkReadMode.ON_OPEN
        var afterMove = AfterMoveAction.LIST
        compose.setContent {
            MailcraneTheme { ReadingContent(markRead, { markRead = it }, afterMove, { afterMove = it }) }
        }
        compose.onNodeWithText("When opened").assertIsSelected()
        compose.onNodeWithText("Back to the list").assertIsSelected()
        compose.onNodeWithText("Only by hand").performClick()
        compose.onNodeWithText("Show next message").performClick()
        assertEquals(MarkReadMode.MANUAL, markRead)
        assertEquals(AfterMoveAction.NEXT, afterMove)
    }
}
