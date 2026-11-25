package app.mailcrane.settings

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** In-memory [AppSettings] for tests. */
class FakeAppSettings(
    cleanLinks: Boolean = true,
    undoSendSeconds: Int = 0,
) : AppSettings {
    private val clean = MutableStateFlow(cleanLinks)
    override val cleanLinks: StateFlow<Boolean> = clean

    override suspend fun setCleanLinks(enabled: Boolean) {
        clean.value = enabled
    }

    private val undo = MutableStateFlow(undoSendSeconds)
    override val undoSendSeconds: StateFlow<Int> = undo

    override suspend fun setUndoSendSeconds(seconds: Int) {
        undo.value = seconds
    }

    private val theme = MutableStateFlow(ThemeMode.SYSTEM)
    override val themeMode: StateFlow<ThemeMode> = theme

    override suspend fun setThemeMode(mode: ThemeMode) {
        theme.value = mode
    }

    private val dynamic = MutableStateFlow(true)
    override val dynamicColor: StateFlow<Boolean> = dynamic

    override suspend fun setDynamicColor(enabled: Boolean) {
        dynamic.value = enabled
    }

    private val darkBodies = MutableStateFlow(false)
    override val darkMessageBodies: StateFlow<Boolean> = darkBodies

    override suspend fun setDarkMessageBodies(enabled: Boolean) {
        darkBodies.value = enabled
    }
}
