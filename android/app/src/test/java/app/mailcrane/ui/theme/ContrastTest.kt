package app.mailcrane.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every text/background colour pair the app draws meets WCAG 2.2 AA: 4.5:1 for text (M6-01).
 * Pairs come from the colour schemes themselves, so Material defaults the brand doesn't set
 * (error and secondary containers) are checked too.
 */
class ContrastTest {
    private fun contrast(
        a: Color,
        b: Color,
    ): Double {
        val la = a.luminance().toDouble()
        val lb = b.luminance().toDouble()
        return (maxOf(la, lb) + 0.05) / (minOf(la, lb) + 0.05)
    }

    /** (name, text, background) for every pair used in the app. */
    private fun pairs(
        s: ColorScheme,
        success: Color,
    ): List<Triple<String, Color, Color>> {
        val backgrounds =
            listOf(
                "background" to s.background,
                "surface" to s.surface,
                "surfaceContainer" to s.surfaceContainer,
                "surfaceContainerHigh" to s.surfaceContainerHigh,
                "surfaceContainerLow" to s.surfaceContainerLow,
            )
        val onBackgrounds =
            listOf(
                "onBackground" to s.onBackground,
                "onSurface" to s.onSurface,
                "onSurfaceVariant" to s.onSurfaceVariant,
                // Coloured text drawn straight on the page: links, headings, error notes, "Saved".
                "primary" to s.primary,
                "error" to s.error,
                "success" to success,
            )
        val onPage =
            backgrounds.flatMap { (bn, bg) -> onBackgrounds.map { (tn, fg) -> Triple("$tn on $bn", fg, bg) } }
        val containers =
            listOf(
                Triple("onPrimary on primary", s.onPrimary, s.primary),
                Triple("onPrimaryContainer on primaryContainer", s.onPrimaryContainer, s.primaryContainer),
                Triple("onSecondaryContainer on secondaryContainer", s.onSecondaryContainer, s.secondaryContainer),
                Triple("onError on error", s.onError, s.error),
                Triple("onErrorContainer on errorContainer", s.onErrorContainer, s.errorContainer),
                // Disabled text is exempt from WCAG, so it isn't listed.
            )
        return onPage + containers
    }

    private fun check(
        scheme: String,
        s: ColorScheme,
        success: Color,
    ): List<String> =
        pairs(s, success).mapNotNull { (name, fg, bg) ->
            val ratio = contrast(fg.compositeOver(bg), bg)
            if (ratio < 4.5) "$scheme: $name is %.2f:1".format(ratio) else null
        }

    @Test
    fun everyTextPairMeetsWcagAa() {
        val failures =
            check("light", MailcraneLightColors, MailcraneLightExtendedColors.success) +
                check("dark", MailcraneDarkColors, MailcraneDarkExtendedColors.success)
        assertTrue(failures.joinToString("\n"), failures.isEmpty())
    }
}
