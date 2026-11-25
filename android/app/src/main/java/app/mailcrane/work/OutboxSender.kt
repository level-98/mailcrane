package app.mailcrane.work

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.CoreEventListener
import app.mailcrane.core.OutboxFailure
import app.mailcrane.core.SyncEngine
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.databasePath
import app.mailcrane.core.outboxAccounts
import app.mailcrane.core.sendOutbox
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.ConcurrentHashMap
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Sends the outbox while the app runs (M2-06): on start-up, when the network comes back, after a
 * message is queued, and when the next retry is due. [OutboxWorker] does the same when the app is in
 * the background.
 */
@Singleton
class OutboxSender
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
    ) {
        private val dbPath = databasePath(context)
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        private val loops = ConcurrentHashMap<Long, Job>()
        private val wakeups = ConcurrentHashMap<Long, Channel<Unit>>()
        private val copies by lazy { SyncEngine(dbPath, NoEvents) }

        /** Call once from the Application: recovers interrupted sends, then starts sending. */
        fun start() {
            scope.launch {
                // Before anything sends: a message left "sending" by a stopped app is checked against the
                // Sent folder. Doing this while another sender is busy could resend its message.
                val accounts = accounts()
                val engine = SyncEngine(dbPath, NoEvents)
                for (account in accounts) {
                    try {
                        engine.recoverOutbox(account)
                    } catch (_: SyncFailure) {
                        // Offline: those messages stay "sending" until the next start.
                    }
                }
                ready.complete(Unit)
                accounts.forEach(::kick)
                if (accounts.isNotEmpty()) OutboxWorker.schedule(context)
            }
            context.getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(
                object : ConnectivityManager.NetworkCallback() {
                    override fun onAvailable(network: Network) = kickAll()
                },
            )
        }

        /** Sends every account's outbox now. */
        fun kickAll() {
            scope.launch { accounts().forEach(::kick) }
        }

        /**
         * Sends [accountId]'s outbox now and keeps retrying until it's empty. A running loop is woken
         * rather than restarted: cancelling it mid-send would leave a message "sending".
         */
        fun kick(accountId: Long) {
            val wake = wakeups.getOrPut(accountId) { Channel(Channel.CONFLATED) }
            if (loops[accountId]?.isActive == true) {
                wake.trySend(Unit)
                return
            }
            loops[accountId] =
                scope.launch {
                    ready.await()
                    while (true) {
                        val status =
                            try {
                                sendOutbox(dbPath, accountId)
                            } catch (_: SyncFailure) {
                                null
                            }
                        if (status != null && status.remaining == 0u) {
                            // Sent: save copies to the Sent folder (M2-07). Failures wait for the next sync.
                            try {
                                copies.replayActions(accountId)
                            } catch (_: SyncFailure) {
                                // Retried by the next sync or send.
                            }
                            break
                        }
                        val now = System.currentTimeMillis()
                        val wait =
                            status?.nextAttemptAt?.let { (it * 1_000 - now).coerceIn(MIN_WAIT_MS, MAX_WAIT_MS) }
                                ?: DEFAULT_WAIT_MS
                        withTimeoutOrNull(wait) { wake.receive() }
                    }
                }
            OutboxWorker.schedule(context)
        }

        private fun accounts(): List<Long> =
            try {
                outboxAccounts(dbPath)
            } catch (_: OutboxFailure) {
                emptyList()
            }

        private object NoEvents : CoreEventListener {
            override fun onEvent(event: CoreEvent) = Unit
        }

        companion object {
            private const val MIN_WAIT_MS = 5_000L
            private const val DEFAULT_WAIT_MS = 30_000L
            private const val MAX_WAIT_MS = 15 * 60_000L

            /** Opens once start-up recovery has run; every sender in the process waits for it. */
            val ready = CompletableDeferred<Unit>()
        }
    }
