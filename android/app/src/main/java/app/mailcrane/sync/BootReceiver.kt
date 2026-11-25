package app.mailcrane.sync

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * After a reboot, brings always-on sync back (M4-05). Starting the app's process is enough: the
 * Application starts [BackgroundMail], which follows the settings. The periodic check needs nothing:
 * WorkManager keeps it across reboots.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        // Nothing to do here: the Application already ran onCreate for this broadcast.
    }
}
