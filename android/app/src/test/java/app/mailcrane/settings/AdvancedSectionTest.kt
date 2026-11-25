package app.mailcrane.settings

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.ui.theme.MailcraneTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class AdvancedSectionTest {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun wifiOnlyIsTheDefaultAndAChoiceIsSaved() =
        runTest {
            val settings = FakeAppSettings()
            val vm = AdvancedSettingsViewModel(settings)
            backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { vm.backgroundDownload.collect {} }
            assertEquals(BackgroundDownloadSetting.WIFI_ONLY, vm.backgroundDownload.value)
            vm.setBackgroundDownload(BackgroundDownloadSetting.NEVER)
            assertEquals(BackgroundDownloadSetting.NEVER, settings.backgroundDownload.value)
            assertEquals(BackgroundDownloadSetting.NEVER, vm.backgroundDownload.value)
        }

    @Test
    fun theSectionShowsTheThreeChoicesAndReportsATap() {
        var chosen: BackgroundDownloadSetting? = null
        compose.setContent {
            MailcraneTheme {
                AdvancedContent(BackgroundDownloadSetting.WIFI_ONLY) { chosen = it }
            }
        }
        compose.onNodeWithText("Advanced").assertIsDisplayed()
        compose.onNodeWithText("Download messages in the background").assertIsDisplayed()
        compose.onNodeWithText("On Wi-Fi only").assertIsSelected()
        compose.onNodeWithText("Never").assertIsDisplayed()
        compose.onNodeWithText("Always (also on mobile data)").performClick()
        assertEquals(BackgroundDownloadSetting.ALWAYS, chosen)
    }
}
