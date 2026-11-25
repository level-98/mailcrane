package app.mailcrane.ui.components

import android.app.Application
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.math.hypot

/** The initials keep their size in the 40 dp circle at any font scale (review of #316). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class SenderAvatarSizeTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theInitialsSizeIgnoresTheFontScale() {
        for (scale in listOf(1f, 1.3f, 2f)) {
            val density = Density(density = 2.75f, fontScale = scale)
            val inDp = with(density) { initialsFontSize(AVATAR_SIZE, density).toDp() }
            assertEquals("at font scale $scale", 16f, inDp.value, 0.01f)
        }
    }

    @Test
    fun wideInitialsLookTheSameAtFontScale2AndFitTheCircle() {
        // sample -> scale -> measured size in dp
        val measured = mutableMapOf<String, MutableMap<Float, Pair<Dp, Dp>>>()
        compose.setContent {
            val base = LocalDensity.current
            for (scale in listOf(1f, 2f)) {
                CompositionLocalProvider(LocalDensity provides Density(base.density, fontScale = scale)) {
                    MailcraneTheme {
                        val density = LocalDensity.current
                        val measurer = rememberTextMeasurer()
                        // The same style the avatar uses.
                        val style = initialsTextStyle(AVATAR_SIZE, density)
                        for (sample in SAMPLES) {
                            val size = measurer.measure(sample, style).size
                            measured.getOrPut(sample) { mutableMapOf() }[scale] =
                                with(density) { size.width.toDp() to size.height.toDp() }
                        }
                    }
                }
            }
        }
        compose.waitForIdle()
        val circle = AVATAR_SIZE.value
        // The account dot: a 14 dp ring at the bottom-right, pushed 4 dp outside the circle's box.
        val ringRadius = 7f
        val ringCentre = circle + 4f - ringRadius
        // The glyphs are about as tall as the 16 dp font size. (Robolectric's line metrics ignore
        // lineHeight, so the measured height is only compared across font scales, not with the circle.)
        val glyphHeight = initialsHeightDp
        for (sample in SAMPLES) {
            val normal = measured.getValue(sample).getValue(1f)
            val large = measured.getValue(sample).getValue(2f)
            assertEquals("$sample width at 2x", normal.first.value, large.first.value, 0.5f)
            assertEquals("$sample height at 2x", normal.second.value, large.second.value, 0.5f)
            val w = large.first.value
            assertTrue("$sample is $w dp wide in a $circle dp circle", w <= circle * 0.75f)
            // The initials' box is centred; its corner nearest the dot must stay outside the ring.
            val corner = circle / 2 + w / 2 to circle / 2 + glyphHeight / 2
            val gap = hypot(ringCentre - corner.first, ringCentre - corner.second) - ringRadius
            assertTrue("$sample's corner is ${"%.1f".format(gap)} dp from the dot", gap > 0f)
        }
    }

    private companion object {
        /** Wide pairs, and the ones that clipped in review. */
        val SAMPLES = listOf("WM", "MW", "DT", "PP")
        val initialsHeightDp = AVATAR_SIZE.value * 0.4f
    }
}
