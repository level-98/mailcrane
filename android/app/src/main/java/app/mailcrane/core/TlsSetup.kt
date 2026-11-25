package app.mailcrane.core

import android.content.Context

/**
 * Lets the Rust core check server certificates with Android's own trust store
 * (rustls-platform-verifier). Must run once before the core makes any TLS connection; without it
 * every sign-in and sync fails (issue #243).
 */
object TlsSetup {
    @Volatile private var done = false

    @Synchronized
    fun init(context: Context) {
        if (done) return
        System.loadLibrary("mailcrane_ffi")
        nativeInit(context.applicationContext)
        done = true
    }

    @JvmStatic private external fun nativeInit(context: Context)
}
