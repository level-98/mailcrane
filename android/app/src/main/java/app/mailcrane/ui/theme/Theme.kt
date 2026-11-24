package app.mailcrane.ui.theme

import android.provider.Settings
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

// Placeholder colours; the generated brand theme comes in M0-10.
private val LightColors = lightColorScheme()
private val DarkColors = darkColorScheme()

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun MailcraneTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val resolver = LocalContext.current.contentResolver
    // Reduced motion: the animator duration scale is 0.
    val reducedMotion =
        remember(resolver) {
            Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
        }
    MaterialExpressiveTheme(
        colorScheme = if (darkTheme) DarkColors else LightColors,
        motionScheme = if (reducedMotion) MotionScheme.standard() else MotionScheme.expressive(),
        content = content,
    )
}
