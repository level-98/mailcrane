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

    private val right = MutableStateFlow(DEFAULT_SWIPE_RIGHT)
    override val swipeRight: StateFlow<SwipeAction> = right

    override suspend fun setSwipeRight(action: SwipeAction) {
        right.value = action
    }

    private val left = MutableStateFlow(DEFAULT_SWIPE_LEFT)
    override val swipeLeft: StateFlow<SwipeAction> = left

    override suspend fun setSwipeLeft(action: SwipeAction) {
        left.value = action
    }

    private val recent = MutableStateFlow<List<String>>(emptyList())
    override val recentSearches: StateFlow<List<String>> = recent

    override suspend fun addRecentSearch(query: String) {
        recent.value = (listOf(query) + recent.value.filter { it != query }).take(MAX_RECENT_SEARCHES)
    }

    override suspend fun clearRecentSearches() {
        recent.value = emptyList()
    }
}
