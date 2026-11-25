package app.mailcrane

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import app.mailcrane.core.LiveUpdatesEngine
import app.mailcrane.core.LiveUpdatesObserver
import app.mailcrane.core.installSecretStore
import app.mailcrane.security.KeystoreSecretStore
import app.mailcrane.work.OutboxSender
import dagger.hilt.android.HiltAndroidApp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import javax.inject.Inject

@HiltAndroidApp
class MailcraneApplication : Application() {
    @Inject lateinit var liveUpdates: LiveUpdatesEngine

    // Lives as long as the process.
    private val appScope = CoroutineScope(SupervisorJob())

    @Inject lateinit var outbox: OutboxSender

    @Inject lateinit var widgets: app.mailcrane.widget.WidgetUpdater

    override fun onCreate() {
        super.onCreate()
        // The core reads and writes credentials only through this store.
        installSecretStore(KeystoreSecretStore(this))
        // New mail arrives within seconds while the app is in the foreground (IMAP IDLE); live
        // updates stop when it goes to the background.
        ProcessLifecycleOwner
            .get()
            .lifecycle
            .addObserver(LiveUpdatesObserver(liveUpdates, appScope, Dispatchers.IO))
        // Recovers sends interrupted by a stopped app, then sends what's queued.
        outbox.start()
        // The home-screen widget follows mail changes (M4-11).
        widgets.start()
        // Attachments opened more than a day ago (M2-04).
        Thread {
            app.mailcrane.ui.message
                .clearOldAttachments(this)
        }.start()
    }
}
