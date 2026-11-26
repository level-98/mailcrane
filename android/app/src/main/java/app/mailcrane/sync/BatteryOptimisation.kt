package app.mailcrane.sync

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings

/**
 * Android's battery optimisation (Doze, App Standby) and always-on sync (M4-05). While the app is
 * optimised, Android may cut its network in the background and is freer to stop the always-on
 * service, so notifications come late. Exempt, the service can also be started again from the
 * background (by the periodic watchdog) after a phone stopped it.
 *
 * Mailcrane doesn't ask for the exemption directly (that needs a permission Google Play limits);
 * it explains why and opens the system's list, where the user turns it off themself.
 */
object BatteryOptimisation {
    /** Whether Android exempts Mailcrane from battery optimisation. */
    fun isExempt(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) ?: false

    /**
     * Opens the system's battery optimisation list, or the app's own settings page on phones that
     * don't have that screen. Returns false if neither could be opened.
     */
    fun openSettings(context: Context): Boolean {
        val candidates =
            listOf(
                Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS),
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
            )
        for (intent in candidates) {
            try {
                context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                return true
            } catch (_: ActivityNotFoundException) {
                // Try the next one.
            } catch (_: SecurityException) {
                // Some phones guard their settings screens; try the next one.
            }
        }
        return false
    }
}
