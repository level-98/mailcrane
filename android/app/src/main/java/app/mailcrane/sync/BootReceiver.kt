package app.mailcrane.sync

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import app.mailcrane.settings.AppSettings
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/** The broadcasts after which always-on sync must be started again. */
internal val RESTART_ACTIONS =
    setOf(
        // The device started.
        Intent.ACTION_BOOT_COMPLETED,
        // The app was updated: Android stops the old version's service and doesn't restart it.
        Intent.ACTION_MY_PACKAGE_REPLACED,
    )

/** Whether always-on sync should be started for the broadcast [action], as [settings] say. */
internal suspend fun restartsAlwaysOn(
    action: String?,
    settings: AppSettings,
): Boolean =
    action in RESTART_ACTIONS &&
        backgroundMode(settings.notificationsEnabled.first(), settings.alwaysOnSync.first()) ==
        BackgroundMode.ALWAYS_ON

/**
 * Brings always-on sync back after a reboot or an app update (M4-05). It starts the service here,
 * while Android allows a foreground service to start from the background because of this
 * broadcast, instead of leaving it to the Application's settings watcher, which might run too
 * late. The `specialUse` type is not one Android 15 forbids from boot receivers. The periodic
 * check needs nothing: WorkManager keeps it across reboots and updates.
 */
class BootReceiver : BroadcastReceiver() {
    @EntryPoint
    @InstallIn(SingletonComponent::class)
    interface Deps {
        fun settings(): AppSettings
    }

    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        if (intent.action !in RESTART_ACTIONS) return
        val app = context.applicationContext
        val settings = EntryPointAccessors.fromApplication(app, Deps::class.java).settings()
        val pending = goAsync()
        CoroutineScope(Dispatchers.Default).launch {
            try {
                // Well inside the 10 seconds a receiver gets.
                val restart = withTimeoutOrNull(SETTINGS_TIMEOUT_MS) { restartsAlwaysOn(intent.action, settings) }
                if (restart == true) AlwaysOnSyncService.start(app)
            } finally {
                pending.finish()
            }
        }
    }

    private companion object {
        const val SETTINGS_TIMEOUT_MS = 5_000L
    }
}
