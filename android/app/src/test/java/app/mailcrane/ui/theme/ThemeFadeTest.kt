package app.mailcrane.ui.theme

import android.app.Application
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** #339: switching between light and dark fades the colours instead of snapping. */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class ThemeFadeTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun coloursFadeFromLightToDark() {
        var dark by mutableStateOf(false)
        var surface = Color.Unspecified
        compose.setContent {
            MailcraneTheme(darkTheme = dark) {
                val shown = MaterialTheme.colorScheme.surface
                SideEffect { surface = shown }
            }
        }
        compose.waitForIdle()
        assertEquals(MailcraneLightColors.surface, surface)

        compose.mainClock.autoAdvance = false
        compose.runOnUiThread {
            dark = true
            Snapshot.sendApplyNotifications()
        }
        compose.mainClock.advanceTimeBy(THEME_FADE_MS / 2L)
        assertNotEquals("part-way, not snapped", MailcraneLightColors.surface, surface)
        assertNotEquals("part-way, not snapped", MailcraneDarkColors.surface, surface)
        compose.mainClock.advanceTimeBy(THEME_FADE_MS.toLong() + 100)
        assertEquals(MailcraneDarkColors.surface, surface)
    }
}
