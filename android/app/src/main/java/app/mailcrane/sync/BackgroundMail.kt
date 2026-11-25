package app.mailcrane.sync

import android.content.Context
import app.mailcrane.settings.AppSettings
import app.mailcrane.work.MailCheckWorker
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import javax.inject.Inject
import javax.inject.Singleton

/** How new mail is checked for in the background (M4-05). */
enum class BackgroundMode {
    /** Notifications are off: nothing runs in the background. */
    NONE,

    /** About every 15 minutes, through WorkManager. */
    PERIODIC,

    /** Always-on: live updates kept running by a foreground service. */
    ALWAYS_ON,
}

/** The background mode for the user's settings. */
fun backgroundMode(
    notifications: Boolean,
    alwaysOn: Boolean,
): BackgroundMode =
    when {
        !notifications -> BackgroundMode.NONE
        alwaysOn -> BackgroundMode.ALWAYS_ON
        else -> BackgroundMode.PERIODIC
    }

/** Starts and stops the background check to follow the settings. */
@Singleton
class BackgroundMail
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
        private val settings: AppSettings,
    ) {
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

        /** Call once from the Application (also after a reboot, through the app starting). */
        fun start() {
            scope.launch {
                combine(settings.notificationsEnabled, settings.alwaysOnSync, ::backgroundMode)
                    .distinctUntilChanged()
                    .collect(::apply)
            }
        }

        private fun apply(mode: BackgroundMode) {
            when (mode) {
                BackgroundMode.NONE -> {
                    AlwaysOnSyncService.stop(context)
                    MailCheckWorker.cancel(context)
                }
                BackgroundMode.PERIODIC -> {
                    AlwaysOnSyncService.stop(context)
                    MailCheckWorker.schedule(context)
                }
                BackgroundMode.ALWAYS_ON -> {
                    MailCheckWorker.cancel(context)
                    AlwaysOnSyncService.start(context)
                }
            }
        }
    }
