package app.mailcrane.ui.theme

import androidx.compose.foundation.BorderStroke
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp

/** Material's alpha for a disabled component's border. */
private const val DISABLED_BORDER_ALPHA = 0.12f

/**
 * An `OutlinedButton`'s border in the scheme's `outline` (#395). Material draws it with
 * `outlineVariant`, the divider colour, which is below WCAG 2.2 SC 1.4.11's 3:1 against the
 * surface. A disabled button keeps Material's faded border: SC 1.4.11 exempts inactive controls.
 */
@Composable
fun outlinedButtonBorder(enabled: Boolean = true): BorderStroke {
    val colors = MaterialTheme.colorScheme
    return BorderStroke(
        1.dp,
        if (enabled) colors.outline else colors.onSurface.copy(alpha = DISABLED_BORDER_ALPHA),
    )
}
