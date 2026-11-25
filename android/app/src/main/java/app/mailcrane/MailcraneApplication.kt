package app.mailcrane

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import app.mailcrane.core.CoreLog
import app.mailcrane.core.LiveUpdatesEngine
import app.mailcrane.core.LiveUpdatesObserver
import app.mailcrane.core.TlsSetup
import app.mailcrane.core.installSecretStore
import app.mailcrane.security.AppLock
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

    @Inject lateinit var appLock: AppLock

    @Inject lateinit var newMail: app.mailcrane.notify.NewMailChecker

    @Inject lateinit var backgroundMail: app.mailcrane.sync.BackgroundMail

    @Inject lateinit var backgroundDownloads: app.mailcrane.sync.BackgroundDownloads

    override fun onCreate() {
        super.onCreate()
        // Before anything connects: the core checks certificates through Android's trust store.
        TlsSetup.init(this)
        // The core's warnings (a failed sync and which step failed) go to logcat as "mailcrane".
        CoreLog.install()
        // Before any screen shows: with the app lock on, the app starts locked (M5-03).
        appLock.start()
        // The core reads and writes credentials only through this store.
        installSecretStore(KeystoreSecretStore(this))
        // New mail arrives within seconds while the app is in the foreground (IMAP IDLE); live
        // updates stop when it goes to the background.
        ProcessLifecycleOwner
            .get()
            .lifecycle
            .addObserver(
                LiveUpdatesObserver(liveUpdates, appScope, Dispatchers.IO) {
                    app.mailcrane.sync.AlwaysOnSyncService.running
                },
            )
        // New mail becomes notifications (M4-05): after live updates, and in the background either
        // every ~15 minutes or always-on, as the settings say.
        newMail.start()
        backgroundMail.start()
        // Recent messages are downloaded after each sync when the network and battery allow (M4-25).
        backgroundDownloads.start()
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
