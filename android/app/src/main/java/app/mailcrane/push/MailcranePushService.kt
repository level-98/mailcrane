package app.mailcrane.push

import app.mailcrane.core.isPushVerification
import app.mailcrane.work.JmapPushWorker
import app.mailcrane.work.PushCheckWorker
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * Receives events from the user's UnifiedPush distributor (M4-05). The connector library checks
 * that each event carries this app's registration token, so another app can't fake one.
 *
 * A push message means "check for mail now", with one exception: a JMAP server's verification
 * (M4-27), which is answered instead. Only the Rust core reads a push's content, size-limited, to
 * tell the two apart; it's never logged. The check talks only to the user's own mail servers.
 */
class MailcranePushService : PushService() {
    private val state get() = UnifiedPushState.get(this)

    override fun onNewEndpoint(
        endpoint: PushEndpoint,
        instance: String,
    ) {
        if (!state.enabled) return
        val keys = endpoint.pubKeySet?.let { WebPushKeys(it.pubKey, it.auth) }
        state.set(PushStatus.Ready(endpoint.url, endpoint.temporary, keys))
        // JMAP accounts get the endpoint as a push subscription (M4-27).
        JmapPushWorker.onNewEndpoint(this)
    }

    override fun onMessage(
        message: PushMessage,
        instance: String,
    ) {
        if (!state.enabled) return
        routePush(
            message.content,
            isVerification = ::isPushVerification,
            verify = { JmapPushWorker.verify(this, it) },
            wake = { PushCheckWorker.enqueue(this) },
        )
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

/** Sends a JMAP verification to be answered, and anything else to a mail check. */
internal fun routePush(
    content: ByteArray,
    isVerification: (ByteArray) -> Boolean,
    verify: (ByteArray) -> Unit,
    wake: () -> Unit,
) {
    if (isVerification(content)) verify(content) else wake()
}

/** The settings screen's terms for the connector's failure reasons. */
internal fun failureKind(reason: FailedReason): FailureKind =
    when (reason) {
        FailedReason.NETWORK -> FailureKind.NETWORK
        FailedReason.ACTION_REQUIRED -> FailureKind.ACTION_REQUIRED
        FailedReason.VAPID_REQUIRED -> FailureKind.NEEDS_SERVER_KEY
        FailedReason.INTERNAL_ERROR -> FailureKind.OTHER
    }
