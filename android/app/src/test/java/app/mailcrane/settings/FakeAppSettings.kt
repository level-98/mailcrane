package app.mailcrane.settings

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** In-memory [AppSettings] for tests. */
class FakeAppSettings(
    cleanLinks: Boolean = true,
) : AppSettings {
    private val clean = MutableStateFlow(cleanLinks)
    override val cleanLinks: StateFlow<Boolean> = clean

    override suspend fun setCleanLinks(enabled: Boolean) {
        clean.value = enabled
    }
}
