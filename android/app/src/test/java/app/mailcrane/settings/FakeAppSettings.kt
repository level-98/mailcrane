package app.mailcrane.settings

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** In-memory [AppSettings] for tests. */
class FakeAppSettings(
    cleanLinks: Boolean = true,
    confirmLinks: Boolean = true,
    undoSendSeconds: Int = 0,
) : AppSettings {
    private val clean = MutableStateFlow(cleanLinks)
    override val cleanLinks: StateFlow<Boolean> = clean

    override suspend fun setCleanLinks(enabled: Boolean) {
        clean.value = enabled
    }

    private val confirm = MutableStateFlow(confirmLinks)
    override val confirmLinks: StateFlow<Boolean> = confirm

    override suspend fun setConfirmLinks(enabled: Boolean) {
        confirm.value = enabled
    }

    private val undo = MutableStateFlow(undoSendSeconds)
    override val undoSendSeconds: StateFlow<Int> = undo

    override suspend fun setUndoSendSeconds(seconds: Int) {
        undo.value = seconds
    }

    private val notify = MutableStateFlow(true)
    override val notificationsEnabled: StateFlow<Boolean> = notify

    override suspend fun setNotificationsEnabled(enabled: Boolean) {
        notify.value = enabled
    }

    private val alwaysOn = MutableStateFlow(false)
    override val alwaysOnSync: StateFlow<Boolean> = alwaysOn

    override suspend fun setAlwaysOnSync(enabled: Boolean) {
        alwaysOn.value = enabled
    }

    private val notifyPrompt = MutableStateFlow(false)
    override val notificationPromptAnswered: StateFlow<Boolean> = notifyPrompt

    override suspend fun setNotificationPromptAnswered() {
        notifyPrompt.value = true
    }

    private val lock = MutableStateFlow(false)
    override val appLockEnabled: StateFlow<Boolean> = lock

    override suspend fun setAppLockEnabled(enabled: Boolean) {
        lock.value = enabled
    }

    private val lockAfter = MutableStateFlow(60)
    override val appLockTimeoutSeconds: StateFlow<Int> = lockAfter

    override suspend fun setAppLockTimeoutSeconds(seconds: Int) {
        lockAfter.value = seconds
    }

    private val lockScreenshots = MutableStateFlow(false)
    override val appLockAllowScreenshots: StateFlow<Boolean> = lockScreenshots

    override suspend fun setAppLockAllowScreenshots(allow: Boolean) {
        lockScreenshots.value = allow
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

    private val contactsAnswered = MutableStateFlow(false)
    override val contactsPromptAnswered: StateFlow<Boolean> = contactsAnswered

    override suspend fun setContactsPromptAnswered() {
        contactsAnswered.value = true
    }

    private val group = MutableStateFlow(true)
    override val groupByConversation: StateFlow<Boolean> = group

    override suspend fun setGroupByConversation(enabled: Boolean) {
        group.value = enabled
    }

    private val lastAccount = MutableStateFlow(0L)
    override val lastAccountId: StateFlow<Long> = lastAccount

    override suspend fun setLastAccountId(id: Long) {
        lastAccount.value = id
    }

    private val hideOutside = MutableStateFlow(false)
    override val hideContentOutsideApp: StateFlow<Boolean> = hideOutside

    override suspend fun setHideContentOutsideApp(hide: Boolean) {
        hideOutside.value = hide
    }

    val density = MutableStateFlow(MailboxDensity.COMFORTABLE)
    override val mailboxDensity: StateFlow<MailboxDensity> = density

    override suspend fun setMailboxDensity(density: MailboxDensity) {
        this.density.value = density
    }

    val photos = MutableStateFlow(false)
    override val contactPhotos: StateFlow<Boolean> = photos

    override suspend fun setContactPhotos(enabled: Boolean) {
        photos.value = enabled
    }
}
