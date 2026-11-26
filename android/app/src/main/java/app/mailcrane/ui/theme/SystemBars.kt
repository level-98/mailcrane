package app.mailcrane.ui.theme

import android.graphics.Color
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge

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
        navigationBarStyle =
            if (dark) {
                SystemBarStyle.dark(DARK_SCRIM)
            } else {
                SystemBarStyle.light(LIGHT_SCRIM, DARK_SCRIM)
            },
    )
}
