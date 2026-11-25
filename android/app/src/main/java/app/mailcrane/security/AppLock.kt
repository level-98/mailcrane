package app.mailcrane.security

import android.os.SystemClock
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import app.mailcrane.settings.AppSettings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import javax.inject.Inject
import javax.inject.Singleton

/** What the screen shows (M5-03). */
enum class LockUi {
    /** Settings not read yet: show nothing, so mail can't flash up before the lock. */
    LOADING,

    /** Cover everything with the lock screen. */
    LOCKED,

    /** Show the app. */
    UNLOCKED,
}

/** The lock settings the window needs: whether to block screenshots and app-switcher previews. */
data class LockWindow(
    val secure: Boolean = false,
)

/**
 * The app lock for the whole process: one instance, driven by [ProcessLifecycleOwner] (so moving
 * between the app's own screens never counts as leaving) and the user's settings. Every entry
 * point opens [app.mailcrane.MainActivity], which covers its content while [ui] is not UNLOCKED.
 */
@Singleton
class AppLock
    @Inject
    constructor(
        private val settings: AppSettings,
    ) : DefaultLifecycleObserver {
        private val state = AppLockState { SystemClock.elapsedRealtime() }
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        private val uiFlow = MutableStateFlow(LockUi.LOADING)
        private val windowFlow = MutableStateFlow(LockWindow())
        private var loaded = false

        val ui: StateFlow<LockUi> = uiFlow.asStateFlow()
        val window: StateFlow<LockWindow> = windowFlow.asStateFlow()

        /** Call once from `Application.onCreate()`. */
        fun start() {
            ProcessLifecycleOwner.get().lifecycle.addObserver(this)
            scope.launch {
                combine(
                    settings.appLockEnabled,
                    settings.appLockTimeoutSeconds,
                    settings.appLockAllowScreenshots,
                ) { enabled, timeout, screenshots -> Triple(enabled, timeout, screenshots) }
                    .collect { (enabled, timeout, screenshots) ->
                        state.configure(enabled, timeout * 1_000L, coldStart = !loaded)
                        loaded = true
                        windowFlow.value = LockWindow(secure = enabled && !screenshots)
                        publish()
                    }
            }
        }

        override fun onStop(owner: LifecycleOwner) {
            state.onBackground()
        }

        override fun onStart(owner: LifecycleOwner) {
            state.onForeground()
            publish()
        }

        /** The system prompt is opening. */
        fun beginAuthentication() = state.beginAuthentication()

        /** The system prompt closed. */
        fun endAuthentication(success: Boolean) {
            state.endAuthentication(success)
            publish()
        }

        private fun publish() {
            if (!loaded) return
            uiFlow.value = if (state.locked) LockUi.LOCKED else LockUi.UNLOCKED
        }
    }
