package app.mailcrane.ui.theme

import android.content.Context
import android.content.ContextWrapper
import android.graphics.Color
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.delay

/** The scrims androidx uses behind a three-button navigation bar, light and dark. */
private val LIGHT_SCRIM = Color.argb(0xe6, 0xFF, 0xFF, 0xFF)
private val DARK_SCRIM = Color.argb(0x80, 0x1b, 0x1b, 0x1b)

/**
 * Draws the app edge to edge with system bar icons for the theme the app is showing (#412): dark icons on the
 * light theme, light icons on the dark one. [dark] is the app's resolved theme, which follows the in-app choice
 * (Settings → Appearance), not only the system's night mode, so a plain `SystemBarStyle.auto` would be wrong
 * whenever the two differ. Call it again whenever [dark] changes.
 */
fun ComponentActivity.applySystemBarStyle(dark: Boolean) {
    enableEdgeToEdge(
        statusBarStyle =
            if (dark) {
                SystemBarStyle.dark(Color.TRANSPARENT)
            } else {
                SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT)
            },
        // `auto` rather than light/dark here: it leaves the system's contrast handling on, so gesture navigation
        // keeps a transparent bar instead of a solid strip, and only a three-button bar gets the scrim. The lambda
        // still decides from the app's theme, not the system's night mode.
        navigationBarStyle = SystemBarStyle.auto(LIGHT_SCRIM, DARK_SCRIM) { dark },
    )
}

/**
 * Keeps the system bar icons on the theme the app is showing (#412): applies [applySystemBarStyle] for [dark]
 * at once, then again on each change. A change waits for half the colour fade ([THEME_FADE_MS]), so the icons
 * flip as the background crosses over rather than before it starts; with animations off they flip at once.
 */
@Composable
fun FollowSystemBars(dark: Boolean) {
    val context = LocalContext.current
    val activity = remember(context) { context.findComponentActivity() } ?: return
    val reducedMotion =
        remember(context) {
            Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
        }
    val first = remember { booleanArrayOf(true) }
    LaunchedEffect(dark) {
        if (!first[0] && !reducedMotion) delay(THEME_FADE_MS / 2L)
        first[0] = false
        activity.applySystemBarStyle(dark)
    }
}

private tailrec fun Context.findComponentActivity(): ComponentActivity? =
    when (this) {
        is ComponentActivity -> this
        is ContextWrapper -> baseContext.findComponentActivity()
        else -> null
    }
