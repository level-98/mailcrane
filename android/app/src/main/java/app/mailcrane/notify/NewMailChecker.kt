package app.mailcrane.notify

import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.NewMailNotification
import app.mailcrane.core.NotificationsCore
import app.mailcrane.core.SyncFailure
import app.mailcrane.settings.AppSettings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Turns new mail into notifications (M4-05). Live updates (in the app, or always-on in the
 * background) report changes as events; this asks the core what's new after each and shows it.
 * The periodic worker hands its results to [show].
 *
 * New mail is always taken from the core, so it's never notified later; it's only shown when the
 * app isn't on screen and notifications are on.
 */
@Singleton
class NewMailChecker
    @Inject
    constructor(
        private val mailbox: MailboxCore,
        private val core: NotificationsCore,
        private val notifier: MailNotifier,
        private val settings: AppSettings,
    ) {
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        private val serial = Mutex()

        /** Call once from the Application. */
        @OptIn(FlowPreview::class)
        fun start() {
            scope.launch {
                mailbox.events
                    .filter { it is CoreEvent.MessagesChanged }
                    .debounce(DEBOUNCE_MS)
                    .collect { checkStored() }
            }
        }

        /** Takes what the core has stored as new and shows it. */
        suspend fun checkStored() {
            serial.withLock {
                val mail =
                    try {
                        core.takeNewMail()
                    } catch (_: SyncFailure) {
                        return
                    }
                show(mail)
            }
        }

        /** Shows [mail] unless the app is on screen or notifications are off. */
        suspend fun show(mail: List<NewMailNotification>) {
            if (mail.isEmpty()) return
            if (!settings.notificationsEnabled.first() || inForeground()) return
            val hide = settings.hideContentOutsideApp.first() || settings.appLockEnabled.first()
            notifier.post(mail, hide)
        }

        private suspend fun inForeground(): Boolean =
            withContext(Dispatchers.Main.immediate) {
                ProcessLifecycleOwner
                    .get()
                    .lifecycle.currentState
                    .isAtLeast(Lifecycle.State.STARTED)
            }

        private companion object {
            const val DEBOUNCE_MS = 1_500L
        }
    }
