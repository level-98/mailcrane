package app.mailcrane.sync

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import app.mailcrane.core.CloseReason
import app.mailcrane.core.UniffiMailboxCore
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import javax.inject.Inject
import javax.inject.Singleton

/** How long the app stays in the background before its reused connections are closed. */
const val BACKGROUND_CLOSE_DELAY_MS = 60_000L

/**
 * Decides when the core's reused mail connections (one per account, issue #294) are closed: at
 * once when the default network changes or is lost, and after [backgroundDelayMs] in the
 * background, unless the app comes back first. [close] runs on [scope]; it may block.
 *
 * It also tells live updates (IMAP IDLE, M4-05) to [reconnect] whenever a default network arrives
 * that isn't the one they connected on: a different one, or one after none. Without that, an
 * IDLE connection left on the old network would go unnoticed until it is re-issued, and after an
 * outage the next try could be minutes away.
 */
class ConnectionCloser(
    private val scope: CoroutineScope,
    private val backgroundDelayMs: Long = BACKGROUND_CLOSE_DELAY_MS,
    private val reconnect: () -> Unit = {},
    private val close: (CloseReason) -> Unit,
) {
    // Touched only from the callbacks, which Android delivers one at a time.
    private var network: Long? = null
    private var hadNetwork = false
    private var pending: Job? = null

    /** The default network is now [id]: a different one than before closes the connections. */
    fun onNetworkAvailable(id: Long) {
        val previous = network
        network = id
        if (previous != null && previous != id) closeNow()
        // The first network seen is the one live updates started on.
        if (hadNetwork && previous != id) scope.launch { reconnect() }
        hadNetwork = true
    }

    /** The default network [id] went away. */
    fun onNetworkLost(id: Long) {
        if (network == id) {
            network = null
            closeNow()
        }
    }

    /** The app went to the background: close after the delay unless it comes back. */
    fun onBackground() {
        pending?.cancel()
        pending =
            scope.launch {
                delay(backgroundDelayMs)
                close(CloseReason.BACKGROUND)
            }
    }

    /** The app is in the foreground again. */
    fun onForeground() {
        pending?.cancel()
        pending = null
    }

    private fun closeNow() {
        scope.launch { close(CloseReason.NETWORK_CHANGED) }
    }
}

/** Hooks [ConnectionCloser] to Android's default network and the app's lifecycle. */
@Singleton
class ReusedConnections
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
        private val core: UniffiMailboxCore,
    ) {
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        private val closer =
            ConnectionCloser(scope, reconnect = core::reconnectLiveUpdates) { core.closeConnections(it) }

        /** Call once from the Application, on the main thread. */
        fun start() {
            context.getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(
                object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) = closer.onNetworkAvailable(network.networkHandle)

                    override fun onLost(network: Network) = closer.onNetworkLost(network.networkHandle)
                },
            )
            ProcessLifecycleOwner.get().lifecycle.addObserver(
                object : DefaultLifecycleObserver {
                    override fun onStart(owner: LifecycleOwner) = closer.onForeground()

                    override fun onStop(owner: LifecycleOwner) = closer.onBackground()
                },
            )
        }
    }
