package app.mailcrane.ui.theme

import android.app.Application
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Column
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Outlined buttons draw their border in `outline`, which meets 3:1, not `outlineVariant` (#395). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class OutlinedButtonBorderTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun anEnabledButtonsBorderIsTheOutlineInBothThemes() {
        var light: BorderStroke? = null
        var dark: BorderStroke? = null
        compose.setContent {
            Column {
                MailcraneTheme(darkTheme = false) { light = outlinedButtonBorder() }
                MailcraneTheme(darkTheme = true) { dark = outlinedButtonBorder() }
            }
        }
        compose.waitForIdle()
        assertEquals(SolidColor(MailcraneLightColors.outline), light?.brush)
        assertEquals(SolidColor(MailcraneDarkColors.outline), dark?.brush)
    }
}
