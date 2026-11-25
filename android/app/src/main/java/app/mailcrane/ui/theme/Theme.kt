package app.mailcrane.ui.theme

import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

val LocalMailcraneExtendedColors =
    compositionLocalOf { MailcraneLightExtendedColors }

/** Whether wallpaper-based dynamic colour (Material You) exists on this device: Android 12+. */
val dynamicColorAvailable: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun MailcraneTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val resolver = context.contentResolver
    // Reduced motion: the animator duration scale is 0.
    val reducedMotion =
        remember(resolver) {
            Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
        }
    MaterialExpressiveTheme(
        colorScheme =
            when {
                // Wallpaper colours on Android 12+, when the user wants them; the brand colours otherwise.
                dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
                    if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
                darkTheme -> MailcraneDarkColors
                else -> MailcraneLightColors
            },
        motionScheme = if (reducedMotion) MotionScheme.standard() else MotionScheme.expressive(),
    ) {
        CompositionLocalProvider(
            LocalMailcraneExtendedColors provides
                if (darkTheme) MailcraneDarkExtendedColors else MailcraneLightExtendedColors,
            content = content,
        )
    }
}
