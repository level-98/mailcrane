package app.mailcrane.ui.account

import android.app.Application
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.AppPasswordGuide
import app.mailcrane.core.AppPasswordProvider
import app.mailcrane.ui.theme.MailcraneTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** M3-05: the app-password guide on the first setup step. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class AppPasswordGuideTest {
    @get:Rule
    val compose = createComposeRule()

    private val icloud =
        AppPasswordGuide(AppPasswordProvider.ICLOUD, "iCloud Mail", "https://account.apple.com/account/manage")

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun typingAListedAddressShowsItsGuide() {
        val vm = ManualSetupViewModel(FakeSetupCore(mapOf("ann@icloud.com" to icloud)) { 1 })
        vm.onEmailChange("ann@icloud.com ")
        assertEquals(icloud, vm.state.value.guide)
        vm.onEmailChange("ann@example.com")
        assertNull(vm.state.value.guide)
    }

    @Test
    fun theGuideShowsStepsAndOpensTheProvidersPage() {
        val opened = mutableListOf<String>()
        val vm = ManualSetupViewModel(FakeSetupCore(mapOf("ann@icloud.com" to icloud)) { 1 })
        vm.onEmailChange("ann@icloud.com")
        compose.setContent {
            CompositionLocalProvider(
                LocalUriHandler provides
                    object : UriHandler {
                        override fun openUri(uri: String) {
                            opened += uri
                        }
                    },
            ) {
                MailcraneTheme { EmailStepContent(vm.state.value, ManualSetupActions()) }
            }
        }
        compose.onNodeWithTag("app-password-guide").assertIsDisplayed()
        compose.onNodeWithText("iCloud Mail needs an app password").assertIsDisplayed()
        compose.onNodeWithText("Create a password, name it Mailcrane, and copy it.").assertIsDisplayed()
        compose.onNodeWithText("App password").assertIsDisplayed()
        compose.onNodeWithText("Open iCloud Mail settings").performClick()
        assertEquals(listOf("https://account.apple.com/account/manage"), opened)
    }

    @Test
    fun otherAddressesShowNoGuide() {
        val vm = ManualSetupViewModel(FakeSetupCore { 1 })
        vm.onEmailChange("ann@example.com")
        compose.setContent { MailcraneTheme { EmailStepContent(vm.state.value, ManualSetupActions()) } }
        compose.onNodeWithTag("app-password-guide").assertDoesNotExist()
        compose.onNodeWithText("Password").assertIsDisplayed()
    }
}
