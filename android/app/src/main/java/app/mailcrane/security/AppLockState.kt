package app.mailcrane.security

/**
 * The app lock's rules (M5-03), with no Android types so they can be unit-tested.
 *
 * - With the lock on, the app starts locked (so it is locked after process death too).
 * - Going to the background starts the clock; coming back after [timeoutMs] or more locks it.
 *   A timeout of 0 means "immediately".
 * - While the system unlock prompt is open the app can briefly go to the background (the device
 *   PIN screen is another app); that must not count, or "immediately" would lock again straight
 *   after every unlock.
 *
 * [now] is a monotonic clock in milliseconds (`SystemClock.elapsedRealtime()` on a device), so
 * changing the phone's time can't shorten the timeout.
 */
class AppLockState(
    private val now: () -> Long,
) {
    /** Whether content must be covered. */
    var locked: Boolean = true
        private set

    private var enabled: Boolean = false
    private var timeoutMs: Long = 0
    private var backgroundSince: Long? = null
    private var authenticating = false

    /**
     * Applies the user's settings. Called once they've loaded at start-up (until then the app shows
     * nothing) and again whenever they change. Turning the lock on doesn't lock the app that's
     * already open: the user has just authenticated to turn it on.
     */
    fun configure(
        enabled: Boolean,
        timeoutMs: Long,
        coldStart: Boolean,
    ) {
        this.timeoutMs = timeoutMs.coerceAtLeast(0)
        if (!enabled) {
            locked = false
        } else if (coldStart) {
            locked = true
        }
        this.enabled = enabled
    }

    /** The app went to the background. */
    fun onBackground() {
        if (!enabled || authenticating || locked) return
        backgroundSince = now()
    }

    /** The app came back to the foreground. */
    fun onForeground() {
        val since = backgroundSince ?: return
        backgroundSince = null
        if (!enabled) return
        val away = now() - since
        // A clock that went backwards (it shouldn't) counts as "long enough".
        if (away < 0 || away >= timeoutMs) locked = true
    }

    /** The system unlock prompt is about to open. */
    fun beginAuthentication() {
        authenticating = true
    }

    /** The prompt closed: successfully ([success]) or not (cancelled, too many tries). */
    fun endAuthentication(success: Boolean) {
        authenticating = false
        backgroundSince = null
        if (success) locked = false
    }
}

/** The "Lock after" choices, in seconds; 0 is "immediately". */
val APP_LOCK_TIMEOUT_CHOICES = listOf(0, 60, 300, 900, 3600)

/** The default "Lock after": 1 minute. */
const val DEFAULT_APP_LOCK_TIMEOUT_SECONDS = 60
