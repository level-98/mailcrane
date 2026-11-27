package app.mailcrane.settings

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import app.mailcrane.security.APP_LOCK_TIMEOUT_CHOICES
import app.mailcrane.security.DEFAULT_APP_LOCK_TIMEOUT_SECONDS
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import java.io.IOException
import javax.inject.Singleton

/**
 * The user's app settings. An interface so view models can be tested with a fake. Settings hold
 * preferences only, never secrets (those go to the Keystore through the secret store).
 */
interface AppSettings {
    /** "Remove tracking from links": strip tracking parameters before opening a link. On by default. */
    val cleanLinks: Flow<Boolean>

    suspend fun setCleanLinks(enabled: Boolean)

    /** "Confirm links before opening": show a preview of a link's destination first. On by default (M5-07). */
    val confirmLinks: Flow<Boolean>

    suspend fun setConfirmLinks(enabled: Boolean)

    /** "Undo send": seconds a sent message waits before going out (0: off). One of [UNDO_SEND_CHOICES]. */
    val undoSendSeconds: Flow<Int>

    suspend fun setUndoSendSeconds(seconds: Int)

    /** Light, dark or as the system says. */
    val themeMode: Flow<ThemeMode>

    suspend fun setThemeMode(mode: ThemeMode)

    /** "Use wallpaper colours" (Material You, Android 12+). On by default. */
    val dynamicColor: Flow<Boolean>

    suspend fun setDynamicColor(enabled: Boolean)

    /** "Dark message bodies": darken HTML mail in dark mode instead of showing it on a light page. Off by default. */
    val darkMessageBodies: Flow<Boolean>

    suspend fun setDarkMessageBodies(enabled: Boolean)

    /** What swiping a message to the right (start to end) does. */
    val swipeRight: Flow<SwipeAction>

    suspend fun setSwipeRight(action: SwipeAction)

    /** What swiping a message to the left (end to start) does. */
    val swipeLeft: Flow<SwipeAction>

    suspend fun setSwipeLeft(action: SwipeAction)

    /** The last few searches, newest first; kept on this device only. */
    val recentSearches: Flow<List<String>>

    /** Puts [query] first in the recent searches (at most [MAX_RECENT_SEARCHES]). */
    suspend fun addRecentSearch(query: String)

    suspend fun clearRecentSearches()

    /** Whether the message list groups messages into conversations (M4-02); on by default. */
    val groupByConversation: Flow<Boolean>

    suspend fun setGroupByConversation(enabled: Boolean)

    /** Show only counts, not senders or subjects, in widgets and notifications (M4-11). */
    val hideContentOutsideApp: Flow<Boolean>

    suspend fun setHideContentOutsideApp(hide: Boolean)

    /** App lock (M5-03): biometrics or the device credential to open the app. Off by default. */
    val appLockEnabled: Flow<Boolean>

    suspend fun setAppLockEnabled(enabled: Boolean)

    /** "Lock after": seconds in the background before the app locks (0: immediately). */
    val appLockTimeoutSeconds: Flow<Int>

    suspend fun setAppLockTimeoutSeconds(seconds: Int)

    /** Allow screenshots and app-switcher previews while the app lock is on. Off by default. */
    val appLockAllowScreenshots: Flow<Boolean>

    suspend fun setAppLockAllowScreenshots(allow: Boolean)

    /** New-mail notifications (M4-05). On by default; Android 13+ also needs the permission. */
    val notificationsEnabled: Flow<Boolean>

    suspend fun setNotificationsEnabled(enabled: Boolean)

    /** "Always-on sync": keep a connection open for instant notifications. Off by default. */
    val alwaysOnSync: Flow<Boolean>

    suspend fun setAlwaysOnSync(enabled: Boolean)

    /** Whether the user was asked for the notification permission (Android 13+); asked once. */
    val notificationPromptAnswered: Flow<Boolean>

    suspend fun setNotificationPromptAnswered()

    /** The account shown last, to open it again at start-up (M4-01); 0 for none. */
    val lastAccountId: Flow<Long>

    suspend fun setLastAccountId(id: Long)

    /** Whether the user was asked about suggestions from their contacts (M2-08); asked once. */
    val contactsPromptAnswered: Flow<Boolean>

    suspend fun setContactsPromptAnswered()

    /** "Download messages in the background" (M4-25). On Wi-Fi only by default. */
    val backgroundDownload: Flow<BackgroundDownloadSetting>

    suspend fun setBackgroundDownload(setting: BackgroundDownloadSetting)

    /** "Mailbox density" (M4-24): Comfortable, with sender pictures, by default. */
    val mailboxDensity: Flow<MailboxDensity>

    suspend fun setMailboxDensity(density: MailboxDensity)

    /** "Use photos from your contacts" (M4-24): off by default; needs the contacts permission. */
    val contactPhotos: Flow<Boolean>

    suspend fun setContactPhotos(enabled: Boolean)

    /** "Mark as read" (M4-15): when an opened message is marked read. When opened, by default. */
    val markReadMode: Flow<MarkReadMode>

    suspend fun setMarkReadMode(mode: MarkReadMode)

    /** "After deleting or moving" (M4-15): what shows next. Back to the list, by default. */
    val afterMoveAction: Flow<AfterMoveAction>

    suspend fun setAfterMoveAction(action: AfterMoveAction)
}

/** When an opened message is marked read (M4-15). */
enum class MarkReadMode {
    /** As soon as it is shown: the behaviour before M4-15. */
    ON_OPEN,

    /** After it has been shown for [MARK_READ_DELAY_MS]; leaving sooner keeps it unread. */
    AFTER_DELAY,

    /** Never by opening it; only "Mark as read". */
    MANUAL,
}

/** How long a message must stay open to be marked read with [MarkReadMode.AFTER_DELAY], in whole seconds. */
const val MARK_READ_DELAY_SECONDS = 2

/** [MARK_READ_DELAY_SECONDS] in milliseconds. Whole seconds, so the Settings label is exact (#514). */
const val MARK_READ_DELAY_MS = MARK_READ_DELAY_SECONDS * 1_000L

/** What shows after archiving, deleting or moving the open message (M4-15). */
enum class AfterMoveAction {
    /** The next message in the list the message was opened from. */
    NEXT,

    /** The previous message in that list. */
    PREVIOUS,

    /** The list itself: the behaviour before M4-15. */
    LIST,
}

/** How tightly the message list packs its rows (M4-24). */
enum class MailboxDensity {
    /** Sender picture, sender, subject and preview. */
    COMFORTABLE,

    /** No picture, subject and preview on one line, tighter padding: more rows on screen. */
    COMPACT,
}

/** When recent messages are downloaded in the background so they open at once (M4-25). */
enum class BackgroundDownloadSetting { WIFI_ONLY, ALWAYS, NEVER }

/** How many recent searches are kept. */
const val MAX_RECENT_SEARCHES = 5

/** What a swipe on a message does (M4-04). */
enum class SwipeAction { NONE, ARCHIVE, DELETE, READ_UNREAD, FLAG, MOVE }

/** Swiping right toggles read; swiping left archives. */
val DEFAULT_SWIPE_RIGHT = SwipeAction.READ_UNREAD
val DEFAULT_SWIPE_LEFT = SwipeAction.ARCHIVE

/** The app's theme choice. */
enum class ThemeMode { SYSTEM, LIGHT, DARK }

/** The "Undo send" delays offered, in seconds; 0 is off. */
val UNDO_SEND_CHOICES = listOf(0, 5, 10, 20, 30)

/** The default "Undo send" delay. */
const val DEFAULT_UNDO_SEND_SECONDS = 10

private val Context.settingsDataStore: DataStore<Preferences> by preferencesDataStore(name = "settings")

/** [AppSettings] kept in a Preferences DataStore. */
class DataStoreAppSettings(
    private val dataStore: DataStore<Preferences>,
) : AppSettings {
    // An unreadable settings file means the defaults, not a crash.
    private val prefs: Flow<Preferences> =
        dataStore.data.catch { e -> if (e is IOException) emit(emptyPreferences()) else throw e }

    override val cleanLinks: Flow<Boolean> = prefs.map { it[CLEAN_LINKS] ?: true }

    override val confirmLinks: Flow<Boolean> = prefs.map { it[CONFIRM_LINKS] ?: true }

    override val groupByConversation: Flow<Boolean> = prefs.map { it[GROUP_THREADS] ?: true }

    override suspend fun setGroupByConversation(enabled: Boolean) {
        dataStore.edit { it[GROUP_THREADS] = enabled }
    }

    override val hideContentOutsideApp: Flow<Boolean> = prefs.map { it[HIDE_OUTSIDE] ?: false }

    override suspend fun setHideContentOutsideApp(hide: Boolean) {
        dataStore.edit { it[HIDE_OUTSIDE] = hide }
    }

    override val appLockEnabled: Flow<Boolean> = prefs.map { it[APP_LOCK] ?: false }

    override suspend fun setAppLockEnabled(enabled: Boolean) {
        dataStore.edit { it[APP_LOCK] = enabled }
    }

    override val appLockTimeoutSeconds: Flow<Int> =
        prefs.map { p ->
            p[APP_LOCK_TIMEOUT]?.takeIf { it in APP_LOCK_TIMEOUT_CHOICES } ?: DEFAULT_APP_LOCK_TIMEOUT_SECONDS
        }

    override suspend fun setAppLockTimeoutSeconds(seconds: Int) {
        if (seconds !in APP_LOCK_TIMEOUT_CHOICES) return
        dataStore.edit { it[APP_LOCK_TIMEOUT] = seconds }
    }

    override val appLockAllowScreenshots: Flow<Boolean> = prefs.map { it[APP_LOCK_SCREENSHOTS] ?: false }

    override suspend fun setAppLockAllowScreenshots(allow: Boolean) {
        dataStore.edit { it[APP_LOCK_SCREENSHOTS] = allow }
    }

    override val notificationsEnabled: Flow<Boolean> = prefs.map { it[NOTIFY] ?: true }

    override suspend fun setNotificationsEnabled(enabled: Boolean) {
        dataStore.edit { it[NOTIFY] = enabled }
    }

    override val alwaysOnSync: Flow<Boolean> = prefs.map { it[ALWAYS_ON] ?: false }

    override suspend fun setAlwaysOnSync(enabled: Boolean) {
        dataStore.edit { it[ALWAYS_ON] = enabled }
    }

    override val notificationPromptAnswered: Flow<Boolean> = prefs.map { it[NOTIFY_PROMPT] ?: false }

    override suspend fun setNotificationPromptAnswered() {
        dataStore.edit { it[NOTIFY_PROMPT] = true }
    }

    override val lastAccountId: Flow<Long> = prefs.map { it[LAST_ACCOUNT] ?: 0L }

    override suspend fun setLastAccountId(id: Long) {
        dataStore.edit { it[LAST_ACCOUNT] = id }
    }

    override val contactsPromptAnswered: Flow<Boolean> = prefs.map { it[CONTACTS_PROMPT] ?: false }

    override suspend fun setContactsPromptAnswered() {
        dataStore.edit { it[CONTACTS_PROMPT] = true }
    }

    override val backgroundDownload: Flow<BackgroundDownloadSetting> =
        prefs.map { p ->
            BackgroundDownloadSetting.entries.firstOrNull { it.name == p[BACKGROUND_DOWNLOAD] }
                ?: BackgroundDownloadSetting.WIFI_ONLY
        }

    override suspend fun setBackgroundDownload(setting: BackgroundDownloadSetting) {
        dataStore.edit { it[BACKGROUND_DOWNLOAD] = setting.name }
    }

    override suspend fun setCleanLinks(enabled: Boolean) {
        dataStore.edit { it[CLEAN_LINKS] = enabled }
    }

    override suspend fun setConfirmLinks(enabled: Boolean) {
        dataStore.edit { it[CONFIRM_LINKS] = enabled }
    }

    // A stored value that isn't one of the choices (an old or edited file) means the default.
    override val undoSendSeconds: Flow<Int> =
        prefs.map { p -> p[UNDO_SEND]?.takeIf { it in UNDO_SEND_CHOICES } ?: DEFAULT_UNDO_SEND_SECONDS }

    override suspend fun setUndoSendSeconds(seconds: Int) {
        if (seconds !in UNDO_SEND_CHOICES) return
        dataStore.edit { it[UNDO_SEND] = seconds }
    }

    override val mailboxDensity: Flow<MailboxDensity> =
        prefs.map { p -> MailboxDensity.entries.firstOrNull { it.name == p[DENSITY] } ?: MailboxDensity.COMFORTABLE }

    override suspend fun setMailboxDensity(density: MailboxDensity) {
        dataStore.edit { it[DENSITY] = density.name }
    }

    override val contactPhotos: Flow<Boolean> = prefs.map { it[CONTACT_PHOTOS] ?: false }

    override suspend fun setContactPhotos(enabled: Boolean) {
        dataStore.edit { it[CONTACT_PHOTOS] = enabled }
    }

    override val markReadMode: Flow<MarkReadMode> =
        prefs.map { p -> MarkReadMode.entries.firstOrNull { it.name == p[MARK_READ] } ?: MarkReadMode.ON_OPEN }

    override suspend fun setMarkReadMode(mode: MarkReadMode) {
        dataStore.edit { it[MARK_READ] = mode.name }
    }

    override val afterMoveAction: Flow<AfterMoveAction> =
        prefs.map { p -> AfterMoveAction.entries.firstOrNull { it.name == p[AFTER_MOVE] } ?: AfterMoveAction.LIST }

    override suspend fun setAfterMoveAction(action: AfterMoveAction) {
        dataStore.edit { it[AFTER_MOVE] = action.name }
    }

    /**
     * The theme just chosen, shown before the settings file has been written: the app used to wait for the
     * write, so the switch lagged behind the tap (#339). Cleared once the file holds it.
     */
    private val pendingTheme = MutableStateFlow<ThemeMode?>(null)

    override val themeMode: Flow<ThemeMode> =
        combine(
            prefs.map { p -> ThemeMode.entries.firstOrNull { it.name == p[THEME] } ?: ThemeMode.SYSTEM },
            pendingTheme,
        ) { stored, pending -> pending ?: stored }.distinctUntilChanged()

    override suspend fun setThemeMode(mode: ThemeMode) {
        pendingTheme.value = mode
        try {
            dataStore.edit { it[THEME] = mode.name }
        } finally {
            // Saved, or the write failed and the stored choice stands.
            pendingTheme.compareAndSet(mode, null)
        }
    }

    override val dynamicColor: Flow<Boolean> = prefs.map { it[DYNAMIC_COLOR] ?: true }

    override suspend fun setDynamicColor(enabled: Boolean) {
        dataStore.edit { it[DYNAMIC_COLOR] = enabled }
    }

    override val darkMessageBodies: Flow<Boolean> = prefs.map { it[DARK_BODIES] ?: false }

    override suspend fun setDarkMessageBodies(enabled: Boolean) {
        dataStore.edit { it[DARK_BODIES] = enabled }
    }

    override val swipeRight: Flow<SwipeAction> = prefs.map { swipe(it[SWIPE_RIGHT]) ?: DEFAULT_SWIPE_RIGHT }

    override suspend fun setSwipeRight(action: SwipeAction) {
        dataStore.edit { it[SWIPE_RIGHT] = action.name }
    }

    override val swipeLeft: Flow<SwipeAction> = prefs.map { swipe(it[SWIPE_LEFT]) ?: DEFAULT_SWIPE_LEFT }

    override suspend fun setSwipeLeft(action: SwipeAction) {
        dataStore.edit { it[SWIPE_LEFT] = action.name }
    }

    override val recentSearches: Flow<List<String>> =
        prefs.map { p -> p[RECENT]?.split('\n')?.filter { it.isNotBlank() }.orEmpty() }

    override suspend fun addRecentSearch(query: String) {
        val clean = query.replace('\n', ' ').trim()
        if (clean.isEmpty()) return
        dataStore.edit { p ->
            val old = p[RECENT]?.split('\n').orEmpty()
            val kept = old.filter { it != clean && it.isNotBlank() }
            p[RECENT] = (listOf(clean) + kept).take(MAX_RECENT_SEARCHES).joinToString("\n")
        }
    }

    override suspend fun clearRecentSearches() {
        dataStore.edit { it.remove(RECENT) }
    }

    private fun swipe(name: String?): SwipeAction? = SwipeAction.entries.firstOrNull { it.name == name }

    private companion object {
        val RECENT = stringPreferencesKey("recent_searches")
        val SWIPE_RIGHT = stringPreferencesKey("swipe_right")
        val SWIPE_LEFT = stringPreferencesKey("swipe_left")
        val CLEAN_LINKS = booleanPreferencesKey("clean_links")
        val CONFIRM_LINKS = booleanPreferencesKey("confirm_links")
        val CONTACTS_PROMPT = booleanPreferencesKey("contacts_prompt_answered")
        val UNDO_SEND = intPreferencesKey("undo_send_seconds")
        val THEME = stringPreferencesKey("theme_mode")
        val DENSITY = stringPreferencesKey("mailbox_density")
        val CONTACT_PHOTOS = booleanPreferencesKey("contact_photos")
        val DYNAMIC_COLOR = booleanPreferencesKey("dynamic_color")
        val DARK_BODIES = booleanPreferencesKey("dark_message_bodies")
        val LAST_ACCOUNT = longPreferencesKey("last_account_id")
        val HIDE_OUTSIDE = booleanPreferencesKey("hide_content_outside_app")
        val GROUP_THREADS = booleanPreferencesKey("group_by_conversation")
        val APP_LOCK = booleanPreferencesKey("app_lock_enabled")
        val NOTIFY = booleanPreferencesKey("notifications_enabled")
        val ALWAYS_ON = booleanPreferencesKey("always_on_sync")
        val NOTIFY_PROMPT = booleanPreferencesKey("notification_prompt_answered")
        val APP_LOCK_TIMEOUT = intPreferencesKey("app_lock_timeout_seconds")
        val APP_LOCK_SCREENSHOTS = booleanPreferencesKey("app_lock_allow_screenshots")
        val BACKGROUND_DOWNLOAD = stringPreferencesKey("background_download")
        val MARK_READ = stringPreferencesKey("mark_read_mode")
        val AFTER_MOVE = stringPreferencesKey("after_move_action")
    }
}

@Module
@InstallIn(SingletonComponent::class)
object SettingsModule {
    @Provides
    @Singleton
    fun provideAppSettings(
        @ApplicationContext context: Context,
    ): AppSettings = DataStoreAppSettings(context.settingsDataStore)
}
