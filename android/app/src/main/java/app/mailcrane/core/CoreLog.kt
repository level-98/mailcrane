package app.mailcrane.core

import android.util.Log

/**
 * Sends the Rust core's warnings and errors to logcat under the tag "mailcrane" (issue #282). The
 * core forwards only WARN and above from its own code, and those lines hold kinds, error names,
 * command labels and ids, never message content or credentials. Run after [TlsSetup.init], which
 * loads the native library.
 */
object CoreLog {
    const val TAG = "mailcrane"

    /** Installs the sink once. Does nothing if the core can't be reached. */
    fun install() {
        runCatching { installLogSink(LogcatSink) }
    }

    private object LogcatSink : LogSink {
        override fun log(
            level: LogLevel,
            line: String,
        ) {
            when (level) {
                LogLevel.ERROR -> Log.e(TAG, line)
                LogLevel.WARN -> Log.w(TAG, line)
            }
        }
    }
}
