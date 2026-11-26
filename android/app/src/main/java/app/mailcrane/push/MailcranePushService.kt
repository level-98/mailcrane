package app.mailcrane.push

import app.mailcrane.work.PushCheckWorker
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * Receives events from the user's UnifiedPush distributor (M4-05). The connector library checks
 * that each event carries this app's registration token, so another app can't fake one.
 *
 * A push message only means "check for mail now": its content is never read, parsed or logged. A
 * gateway that watches the user's mailbox sends it; the check itself talks only to the user's own
 * mail servers, as always.
 */
class MailcranePushService : PushService() {
    private val state get() = UnifiedPushState.get(this)

    override fun onNewEndpoint(
        endpoint: PushEndpoint,
        instance: String,
    ) {
        if (!state.enabled) return
        state.set(PushStatus.Ready(endpoint.url, endpoint.temporary))
    }

    override fun onMessage(
        message: PushMessage,
        instance: String,
    ) {
        if (!state.enabled) return
        PushCheckWorker.enqueue(this)
    }

    override fun onRegistrationFailed(
        reason: FailedReason,
        instance: String,
    ) {
        if (!state.enabled) return
        state.set(PushStatus.Failed(failureKind(reason)))
    }

    override fun onUnregistered(instance: String) {
        if (!state.enabled) return
        state.set(PushStatus.Failed(FailureKind.UNREGISTERED))
    }
}

/** The settings screen's terms for the connector's failure reasons. */
internal fun failureKind(reason: FailedReason): FailureKind =
    when (reason) {
        FailedReason.NETWORK -> FailureKind.NETWORK
        FailedReason.ACTION_REQUIRED -> FailureKind.ACTION_REQUIRED
        FailedReason.VAPID_REQUIRED -> FailureKind.NEEDS_SERVER_KEY
        FailedReason.INTERNAL_ERROR -> FailureKind.OTHER
    }
