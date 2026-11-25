package app.mailcrane.core

import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Starts and stops the core's live updates (IMAP IDLE, or polling) for each account. An interface
 * so [LiveUpdatesObserver] can be tested with a fake. The calls block: run them off the main
 * thread.
 */
interface LiveUpdatesEngine {
    /** Every account's id, or none if the database cannot be read. */
    fun accountIds(): List<Long>

    /** Starts live updates for [accountId]; returns whether they are running. */
    fun start(accountId: Long): Boolean

    /** Stops live updates for [accountId], within about 2 seconds. */
    fun stop(accountId: Long)
}

/**
 * Keeps live updates running while the app is in the foreground. Register it with
 * `ProcessLifecycleOwner`: it starts every account's live updates on `ON_START` and stops them on
 * `ON_STOP`. New mail then appears within seconds while the app is open, and nothing runs in the
 * background (that is M4-05, push notifications).
 */
class LiveUpdatesObserver(
    private val engine: LiveUpdatesEngine,
    private val scope: CoroutineScope,
    dispatcher: CoroutineDispatcher,
    /** True while something else (always-on sync, M4-05) needs them kept on in the background. */
    private val keepInBackground: () -> Boolean = { false },
) : DefaultLifecycleObserver {
    // One call at a time, in order, so a quick background-foreground switch never stops updates
    // after they were started again.
    private val serial = dispatcher.limitedParallelism(1)

    // Only touched on [serial].
    private val running = mutableSetOf<Long>()

    override fun onStart(owner: LifecycleOwner) = onForeground()

    override fun onStop(owner: LifecycleOwner) = onBackground()

    /** The app came to the foreground: start live updates for every account. */
    fun onForeground() {
        scope.launch(serial) {
            for (id in engine.accountIds()) {
                if (engine.start(id)) running += id
            }
        }
    }

    /** The app went to the background: stop every live update that was started. */
    fun onBackground() {
        scope.launch(serial) {
            if (keepInBackground()) return@launch
            for (id in running) engine.stop(id)
            running.clear()
        }
    }
}
