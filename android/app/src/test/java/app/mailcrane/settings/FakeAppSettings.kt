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
}
