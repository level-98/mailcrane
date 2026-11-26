package app.mailcrane.push

import android.app.Application
import android.content.Intent
import android.content.pm.PackageManager
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.work.NetworkType
import app.mailcrane.R
import app.mailcrane.settings.PushSettings
import app.mailcrane.ui.theme.MailcraneTheme
import app.mailcrane.work.PushCheckWorker
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.data.PushEndpoint

/** Push wake-ups through UnifiedPush (M4-05): state, the service's callbacks, and the settings rows. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class UnifiedPushTest {
    @get:Rule
    val compose = createComposeRule()

    private val context: Application = RuntimeEnvironment.getApplication()

    @Before
    fun setUp() = UnifiedPushState.forgetForTests()

    @After
    fun tearDown() = UnifiedPushState.forgetForTests()

    @Test
    fun offByDefaultAndTheAddressSurvivesAProcessRestart() {
        val state = UnifiedPushState.get(context)
        assertEquals(PushStatus.Off, state.status.value)
        assertFalse(state.enabled)
        state.set(PushStatus.Ready("https://push.example.org/up/abc", temporary = true))
        UnifiedPushState.forgetForTests()
        val again = UnifiedPushState.get(context)
        assertEquals(PushStatus.Ready("https://push.example.org/up/abc", temporary = true), again.status.value)
        assertTrue(again.enabled)
        again.set(PushStatus.Failed(FailureKind.NEEDS_SERVER_KEY))
        UnifiedPushState.forgetForTests()
        assertEquals(PushStatus.Failed(FailureKind.NEEDS_SERVER_KEY), UnifiedPushState.get(context).status.value)
    }

    @Test
    fun theServiceRecordsTheAddressOnlyWhileTurnedOn() {
        val service = Robolectric.buildService(MailcranePushService::class.java).create().get()
        val state = UnifiedPushState.get(context)
        // A late answer after the user turned it off is ignored.
        service.onNewEndpoint(PushEndpoint("https://push.example.org/up/1", null, false), "default")
        assertEquals(PushStatus.Off, state.status.value)

        state.set(PushStatus.Registering)
        service.onNewEndpoint(PushEndpoint("https://push.example.org/up/1", null, false), "default")
        assertEquals(PushStatus.Ready("https://push.example.org/up/1", temporary = false), state.status.value)
        service.onRegistrationFailed(FailedReason.VAPID_REQUIRED, "default")
        assertEquals(PushStatus.Failed(FailureKind.NEEDS_SERVER_KEY), state.status.value)
        service.onUnregistered("default")
        assertEquals(PushStatus.Failed(FailureKind.UNREGISTERED), state.status.value)
    }

    @Test
    fun everyFailureReasonHasAnExplanation() {
        assertEquals(FailureKind.NETWORK, failureKind(FailedReason.NETWORK))
        assertEquals(FailureKind.ACTION_REQUIRED, failureKind(FailedReason.ACTION_REQUIRED))
        assertEquals(FailureKind.NEEDS_SERVER_KEY, failureKind(FailedReason.VAPID_REQUIRED))
        assertEquals(FailureKind.OTHER, failureKind(FailedReason.INTERNAL_ERROR))
    }

    @Test
    fun aWakeUpIsAnExpeditedCheckThatNeedsTheNetwork() {
        val spec = PushCheckWorker.request().workSpec
        assertTrue(spec.expedited)
        assertEquals(NetworkType.CONNECTED, spec.constraints.requiredNetworkType)
    }

    @Test
    fun theServiceIsPrivateAndReceivesConnectorEvents() {
        val found =
            context.packageManager.queryIntentServices(
                Intent("org.unifiedpush.android.connector.PUSH_EVENT").setPackage(context.packageName),
                PackageManager.GET_META_DATA,
            )
        val service = found.map { it.serviceInfo }.single { it.name == MailcranePushService::class.java.name }
        assertFalse(service.exported)
    }

    @Test
    fun withoutADistributorTheSwitchExplainsAndStaysOff() {
        compose.setContent {
            MailcraneTheme { PushSettings(PushStatus.Off, hasDistributor = false, onToggle = {}, onCopy = {}) }
        }
        compose.onNodeWithText(context.getString(R.string.push_no_distributor)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.push_title)).assertIsDisplayed()
        compose.onNode(androidx.compose.ui.test.isToggleable()).assertIsNotEnabled()
    }

    @Test
    fun aReadyAddressIsShownAndCopied() {
        val copied = mutableListOf<String>()
        compose.setContent {
            MailcraneTheme {
                PushSettings(
                    PushStatus.Ready("https://push.example.org/up/xyz", temporary = false),
                    hasDistributor = true,
                    onToggle = {},
                    onCopy = { copied += it },
                )
            }
        }
        compose.onNodeWithText("https://push.example.org/up/xyz").assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.push_copy)).performClick()
        assertEquals(listOf("https://push.example.org/up/xyz"), copied)
    }

    @Test
    fun turningItOffWorksEvenWithoutADistributor() {
        val toggles = mutableListOf<Boolean>()
        compose.setContent {
            MailcraneTheme {
                PushSettings(
                    PushStatus.Failed(FailureKind.UNREGISTERED),
                    hasDistributor = false,
                    onToggle = { toggles += it },
                    onCopy = {},
                )
            }
        }
        compose.onNodeWithText(context.getString(R.string.push_failed_unregistered)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.push_title)).performClick()
        assertEquals(listOf(false), toggles)
    }
}
