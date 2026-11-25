package app.mailcrane

import android.app.Application
import app.mailcrane.core.installSecretStore
import app.mailcrane.security.KeystoreSecretStore
import app.mailcrane.work.OutboxSender
import dagger.hilt.android.HiltAndroidApp
import javax.inject.Inject

@HiltAndroidApp
class MailcraneApplication : Application() {
    @Inject lateinit var outbox: OutboxSender

    override fun onCreate() {
        super.onCreate()
        // The core reads and writes credentials only through this store.
        installSecretStore(KeystoreSecretStore(this))
        // Recovers sends interrupted by a stopped app, then sends what's queued.
        outbox.start()
        // Attachments opened more than a day ago (M2-04).
        Thread {
            app.mailcrane.ui.message
                .clearOldAttachments(this)
        }.start()
    }
}
