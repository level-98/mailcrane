package app.mailcrane.push

import android.app.Activity
import android.content.Context
import app.mailcrane.work.JmapPushWorker
import org.unifiedpush.android.connector.UnifiedPush

/**
 * Turns push wake-ups on and off (M4-05). Off by default: it needs a UnifiedPush distributor app
 * and, for IMAP accounts, a gateway on the user's side that watches their mailbox and posts to the
 * address (docs/unifiedpush.md). JMAP accounts are subscribed with their server (M4-27).
 *
 * The periodic check keeps running alongside, so nothing is missed if the gateway is down.
 */
object UnifiedPushControl {
    /** Shown by some distributors next to the registration. */
    private const val LABEL = "Mailcrane: new mail"

    /** Whether any distributor app is installed. */
    fun hasDistributor(context: Context): Boolean = UnifiedPush.getDistributors(context).isNotEmpty()

    /**
     * Uses the distributor the user already chose, or the phone's default one (asking the user if
     * there are several), then registers. The result arrives in [MailcranePushService].
     */
    fun turnOn(activity: Activity) {
        val state = UnifiedPushState.get(activity)
        state.set(PushStatus.Registering)
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { ok ->
            if (ok) {
                UnifiedPush.register(activity, messageForDistributor = LABEL)
            } else {
                state.set(PushStatus.NoDistributor)
            }
        }
    }

    /**
     * Ends the registration with the distributor, removes the JMAP push subscriptions (M4-27) and
     * forgets the address.
     */
    fun turnOff(context: Context) {
        JmapPushWorker.turnedOff(context)
        UnifiedPush.unregister(context)
        UnifiedPushState.get(context).set(PushStatus.Off)
    }
}
