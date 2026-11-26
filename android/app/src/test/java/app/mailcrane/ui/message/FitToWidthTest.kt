package app.mailcrane.ui.message

import android.app.Application
import android.webkit.WebView
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * #342: the message web view zooms wide mail out to the screen's width. Robolectric's web view doesn't
 * lay pages out, so this checks the settings; the scaling itself is checked on the emulator.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class FitToWidthTest {
    @Test
    fun wideMailIsZoomedOutToFitAndTheSafetySettingsStay() {
        val view = WebView(ApplicationProvider.getApplicationContext())
        fitToWidth(view)
        lockDown(view, allowRemote = false)
        with(view.settings) {
            assertTrue(useWideViewPort)
            assertTrue(loadWithOverviewMode)
            // Fitting changes no safety setting.
            assertFalse(javaScriptEnabled)
            assertFalse(allowFileAccess)
            assertFalse(allowContentAccess)
            assertTrue(blockNetworkLoads)
            assertFalse(builtInZoomControls)
        }
    }
}
