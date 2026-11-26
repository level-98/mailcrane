package app.mailcrane.sync

import android.app.Application
import android.os.PowerManager
import android.provider.Settings
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import org.robolectric.RuntimeEnvironment
import app.mailcrane.R
import app.mailcrane.settings.BatteryExplanation
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class BatteryOptimisationTest {
    @get:Rule
    val compose = createComposeRule()

    private val context: Application = RuntimeEnvironment.getApplication()

    @Test
    fun followsWhatAndroidSays() {
        val power = shadowOf(context.getSystemService(PowerManager::class.java))
        power.setIgnoringBatteryOptimizations(context.packageName, false)
        assertFalse(BatteryOptimisation.isExempt(context))
        power.setIgnoringBatteryOptimizations(context.packageName, true)
        assertTrue(BatteryOptimisation.isExempt(context))
    }

    @Test
    fun theExplanationOpensTheSystemListAndCloses() {
        var dismissed = false
        compose.setContent { MailcraneTheme { BatteryExplanation(onDismiss = { dismissed = true }) } }
        compose.onNodeWithText(context.getString(R.string.battery_explain_title)).assertIsDisplayed()
        compose.onNodeWithText(context.getString(R.string.battery_explain_open)).performClick()
        val opened = shadowOf(context).nextStartedActivity
        // The user decides on the system's own screen; Mailcrane asks for no permission.
        assertEquals(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS, opened?.action)
        assertTrue(dismissed)
    }

    @Test
    fun notNowJustCloses() {
        var dismissed = false
        compose.setContent { MailcraneTheme { BatteryExplanation(onDismiss = { dismissed = true }) } }
        compose.onNodeWithText(context.getString(R.string.battery_explain_later)).performClick()
        assertTrue(dismissed)
        assertEquals(null, shadowOf(context).nextStartedActivity)
    }
}
