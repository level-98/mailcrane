package app.mailcrane.push

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Where push wake-ups (UnifiedPush, M4-05) stand. */
sealed interface PushStatus {
    /** Off (the default). */
    data object Off : PushStatus

    /** Asked the distributor for an address; waiting for its answer. */
    data object Registering : PushStatus

    /** The distributor gave this address: messages sent to it wake Mailcrane. */
    data class Ready(
        val endpoint: String,
        /** The distributor's own backend is down and this address is a stand-in that may change. */
        val temporary: Boolean,
        /** Keys for encrypted pushes (RFC 8291), if the distributor gave them (M4-27). */
        val keys: WebPushKeys? = null,
    ) : PushStatus

    /** No distributor app could be used. */
    data object NoDistributor : PushStatus

    /** The distributor refused; [reason] is one of [FailureKind]. */
    data class Failed(
        val reason: FailureKind,
    ) : PushStatus
}

/**
 * The Web Push keys the distributor gave (RFC 8291), base64url: the public key, and the auth
 * secret JMAP servers need to encrypt pushes to this phone. Kept in the app's private
 * preferences, as the connector library keeps them; never logged.
 */
data class WebPushKeys(
    val p256dh: String,
    val auth: String,
)

/** Why registering failed, in the terms the settings screen explains. */
enum class FailureKind {
    /** No network: tried again when it's back, or by turning the switch off and on. */
    NETWORK,

    /** The user has to fix something in the distributor app (sign in, say). */
    ACTION_REQUIRED,

    /** The distributor only accepts servers that identify themselves (VAPID); Mailcrane has none. */
    NEEDS_SERVER_KEY,

    /** Anything else. */
    OTHER,

    /** The distributor ended the registration (it was uninstalled, or the user removed it there). */
    UNREGISTERED,
}

/**
 * The push wake-up state, kept in its own small preferences file so the distributor's callbacks
 * (which can arrive with no screen open) and the settings screen see the same thing.
 *
 * The endpoint is a capability address (anyone who has it can wake the phone, nothing more). It
 * is never logged.
 */
class UnifiedPushState private constructor(
    private val prefs: SharedPreferences,
) {
    private val state = MutableStateFlow(read())

    val status: StateFlow<PushStatus> = state.asStateFlow()

    /** Whether the user turned push wake-ups on. */
    val enabled: Boolean get() = prefs.getBoolean(KEY_ENABLED, false)

    fun set(status: PushStatus) {
        val edit = prefs.edit().clear()
        edit.putBoolean(KEY_ENABLED, status != PushStatus.Off)
        when (status) {
            PushStatus.Off, PushStatus.Registering, PushStatus.NoDistributor -> Unit
            is PushStatus.Ready -> {
                edit.putString(KEY_ENDPOINT, status.endpoint)
                edit.putBoolean(KEY_TEMPORARY, status.temporary)
                status.keys?.let {
                    edit.putString(KEY_P256DH, it.p256dh)
                    edit.putString(KEY_AUTH, it.auth)
                }
            }
            is PushStatus.Failed -> edit.putString(KEY_FAILURE, status.reason.name)
        }
        edit.putString(KEY_KIND, kindOf(status))
        edit.apply()
        state.value = status
    }

    private fun read(): PushStatus {
        if (!prefs.getBoolean(KEY_ENABLED, false)) return PushStatus.Off
        return when (prefs.getString(KEY_KIND, null)) {
            KIND_READY ->
                prefs
                    .getString(
                        KEY_ENDPOINT,
                        null,
                    )?.let { PushStatus.Ready(it, prefs.getBoolean(KEY_TEMPORARY, false), readKeys()) }
                    ?: PushStatus.Registering
            KIND_FAILED ->
                PushStatus.Failed(
                    FailureKind.entries.firstOrNull { it.name == prefs.getString(KEY_FAILURE, null) }
                        ?: FailureKind.OTHER,
                )
            KIND_NO_DISTRIBUTOR -> PushStatus.NoDistributor
            else -> PushStatus.Registering
        }
    }

    private fun readKeys(): WebPushKeys? {
        val p256dh = prefs.getString(KEY_P256DH, null) ?: return null
        val auth = prefs.getString(KEY_AUTH, null) ?: return null
        return WebPushKeys(p256dh, auth)
    }

    private fun kindOf(status: PushStatus): String =
        when (status) {
            PushStatus.Off -> KIND_OFF
            PushStatus.Registering -> KIND_REGISTERING
            is PushStatus.Ready -> KIND_READY
            PushStatus.NoDistributor -> KIND_NO_DISTRIBUTOR
            is PushStatus.Failed -> KIND_FAILED
        }

    companion object {
        private const val FILE = "unifiedpush"
        private const val KEY_ENABLED = "enabled"
        private const val KEY_KIND = "kind"
        private const val KEY_ENDPOINT = "endpoint"
        private const val KEY_TEMPORARY = "temporary"
        private const val KEY_FAILURE = "failure"
        private const val KEY_P256DH = "p256dh"
        private const val KEY_AUTH = "auth"
        private const val KIND_OFF = "off"
        private const val KIND_REGISTERING = "registering"
        private const val KIND_READY = "ready"
        private const val KIND_NO_DISTRIBUTOR = "no-distributor"
        private const val KIND_FAILED = "failed"

        @Volatile private var instance: UnifiedPushState? = null

        /** Forgets the state object, so a test starts from fresh preferences. */
        internal fun forgetForTests() {
            instance = null
        }

        /** The one state for this process. */
        fun get(context: Context): UnifiedPushState =
            instance ?: synchronized(this) {
                instance ?: UnifiedPushState(
                    context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE),
                ).also { instance = it }
            }
    }
}
