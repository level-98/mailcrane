package app.mailcrane

import android.app.Application
import app.mailcrane.core.installSecretStore
import app.mailcrane.security.KeystoreSecretStore
import dagger.hilt.android.HiltAndroidApp

@HiltAndroidApp
class MailcraneApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        // The core reads and writes credentials only through this store.
        installSecretStore(KeystoreSecretStore(this))
    }
}
