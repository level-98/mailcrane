package app.mailcrane.ui.theme

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.core.view.WindowCompat
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** System bar icons follow the theme the app shows, not only the system's night mode (#412). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class SystemBarsTest {
    private fun activity() = Robolectric.buildActivity(ComponentActivity::class.java).setup().get()

    private fun ComponentActivity.lightStatusIcons() =
        WindowCompat.getInsetsController(window, window.decorView).isAppearanceLightStatusBars

    private fun ComponentActivity.lightNavigationIcons() =
        WindowCompat.getInsetsController(window, window.decorView).isAppearanceLightNavigationBars

    @Test
    fun theLightThemeGetsDarkIcons() {
        // "Appearance light status bars" means dark icons, for a light background.
        val activity = activity()
        activity.applySystemBarStyle(dark = false)
        assertTrue(activity.lightStatusIcons())
        assertTrue(activity.lightNavigationIcons())
    }

    @Test
    fun theDarkThemeGetsLightIcons() {
        val activity = activity()
        activity.applySystemBarStyle(dark = true)
        assertFalse(activity.lightStatusIcons())
        assertFalse(activity.lightNavigationIcons())
    }

    @Test
    fun switchingThemeSwitchesTheIcons() {
        // The in-app choice can change while the app is open (Settings → Appearance); the icons follow each time.
        val activity = activity()
        activity.applySystemBarStyle(dark = true)
        assertFalse(activity.lightStatusIcons())
        activity.applySystemBarStyle(dark = false)
        assertTrue(activity.lightStatusIcons())
        activity.applySystemBarStyle(dark = true)
        assertFalse(activity.lightStatusIcons())
    }
}
