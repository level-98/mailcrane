package app.mailcrane.sync

import android.app.Application
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import org.robolectric.RuntimeEnvironment
import app.mailcrane.settings.FakeAppSettings
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Always-on sync comes back after a reboot or an app update, and its service is declared the way
 * Android 14+ requires (M4-05). Robolectric reads the merged manifest.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class AlwaysOnRestartTest {
    private val context: Application = RuntimeEnvironment.getApplication()

    @Test
    fun restartsAfterRebootAndUpdateOnlyWhenAlwaysOnIsChosen() =
        runTest {
            val settings = FakeAppSettings()
            settings.setNotificationsEnabled(true)
            settings.setAlwaysOnSync(true)
            assertTrue(restartsAlwaysOn(Intent.ACTION_BOOT_COMPLETED, settings))
            assertTrue(restartsAlwaysOn(Intent.ACTION_MY_PACKAGE_REPLACED, settings))
            assertFalse(restartsAlwaysOn(Intent.ACTION_TIME_CHANGED, settings))
            assertFalse(restartsAlwaysOn(null, settings))

            settings.setAlwaysOnSync(false)
            assertFalse(restartsAlwaysOn(Intent.ACTION_BOOT_COMPLETED, settings))

            settings.setAlwaysOnSync(true)
            settings.setNotificationsEnabled(false)
            assertFalse(restartsAlwaysOn(Intent.ACTION_BOOT_COMPLETED, settings))
        }

    @Test
    fun theReceiverIsRegisteredForBootAndUpdates() {
        for (action in RESTART_ACTIONS) {
            val found =
                context.packageManager
                    .queryBroadcastReceivers(Intent(action).setPackage(context.packageName), 0)
                    .map { it.activityInfo.name }
            assertTrue("$action: $found", BootReceiver::class.java.name in found)
        }
    }

    @Test
    fun theServiceIsASpecialUseForegroundServiceThatOutlivesTheTask() {
        val info =
            context.packageManager.getServiceInfo(
                ComponentName(context, AlwaysOnSyncService::class.java),
                PackageManager.GET_META_DATA,
            )
        // Android 14+ needs a type; specialUse has no daily time limit (dataSync has 6 hours on
        // Android 15) and may be started from a boot receiver.
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE, info.foregroundServiceType)
        // Swiping the app away from recents must not stop it.
        assertEquals(0, info.flags and ServiceInfo.FLAG_STOP_WITH_TASK)
        assertFalse(info.exported)
    }
}
