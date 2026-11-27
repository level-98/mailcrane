package app.mailcrane

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import app.mailcrane.contacts.LocalContactPhotos
import app.mailcrane.security.AppLock
import app.mailcrane.security.LockScreen
import app.mailcrane.security.LockUi
import app.mailcrane.settings.ThemeMode
import app.mailcrane.ui.MailHome
import app.mailcrane.ui.account.AccountsViewModel
import app.mailcrane.ui.account.ManualSetupScreen
import app.mailcrane.ui.components.WelcomeState
import app.mailcrane.ui.theme.FollowSystemBars
import app.mailcrane.ui.theme.LocalDarkMessageBodies
import app.mailcrane.ui.theme.LocalDarkTheme
import app.mailcrane.ui.theme.LocalMailboxDensity
import app.mailcrane.ui.theme.MailcraneTheme
import app.mailcrane.ui.theme.ThemeViewModel
import app.mailcrane.ui.theme.applySystemBarStyle
import app.mailcrane.widget.EXTRA_ACCOUNT_ID
import app.mailcrane.widget.EXTRA_MESSAGE_ID
import app.mailcrane.work.SnoozeWaker
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import javax.inject.Inject

// A FragmentActivity because the system unlock prompt (androidx.biometric) needs one.
@AndroidEntryPoint
class MainActivity : FragmentActivity() {
    @Inject lateinit var snoozeWaker: SnoozeWaker

    @Inject lateinit var appLock: AppLock

    @Inject lateinit var settings: app.mailcrane.settings.AppSettings

    /** A message to open, from a widget tap (M4-11): (account, message). */
    private var openRequest by mutableStateOf<Pair<Long, Long>?>(null)

    private fun readOpenRequest(intent: Intent?) {
        val message = intent?.getLongExtra(EXTRA_MESSAGE_ID, 0L) ?: 0L
        val account = intent?.getLongExtra(EXTRA_ACCOUNT_ID, 0L) ?: 0L
        if (message > 0 && account > 0) openRequest = account to message
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        readOpenRequest(intent)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Edge to edge from the first frame, with the system's night mode as the best guess until the app's
        // own theme (which may differ) is known below (#412).
        val systemDark =
            resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK ==
                android.content.res.Configuration.UI_MODE_NIGHT_YES
        applySystemBarStyle(systemDark)
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) readOpenRequest(intent)
        // With the app lock on, no screenshots and a blank app-switcher preview (M5-03), unless the
        // user allowed screenshots.
        lifecycleScope.launch {
            appLock.window.collect { w ->
                if (w.secure) {
                    window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
                } else {
                    window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                }
            }
        }
        // Snoozed messages come back when the app opens and on the minute while it's shown (M4-06).
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.RESUMED) {
                while (true) {
                    snoozeWaker.wake()
                    delay(SNOOZE_CHECK_MS)
                }
            }
        }
        setContent {
            val themeModel: ThemeViewModel = hiltViewModel()
            val appearance by themeModel.appearance.collectAsStateWithLifecycle()
            val density by themeModel.mailboxDensity.collectAsStateWithLifecycle()
            val contactPhotos by themeModel.contactPhotos.collectAsStateWithLifecycle()
            val dark =
                when (appearance.mode) {
                    ThemeMode.SYSTEM -> isSystemInDarkTheme()
                    ThemeMode.LIGHT -> false
                    ThemeMode.DARK -> true
                }
            // Status and navigation bar icons readable on the theme actually shown, including the in-app
            // Light/Dark choice (#412).
            FollowSystemBars(dark)
            MailcraneTheme(darkTheme = dark, dynamicColor = appearance.dynamicColor) {
                CompositionLocalProvider(
                    LocalDarkTheme provides dark,
                    LocalDarkMessageBodies provides appearance.darkMessageBodies,
                    LocalMailboxDensity provides density,
                    LocalContactPhotos provides contactPhotos,
                ) {
                    // Nothing from the mailbox is composed until the lock allows it, whatever opened
                    // the app (launcher, widget tap): a widget's message opens after unlocking.
                    val lock by appLock.ui.collectAsStateWithLifecycle()
                    when (lock) {
                        LockUi.LOADING -> Surface(color = MaterialTheme.colorScheme.surface) {}
                        LockUi.LOCKED -> LockScreen(appLock)
                        LockUi.UNLOCKED -> AppContent()
                    }
                }
            }
        }
    }

    @Composable
    private fun AppContent() {
        // The saved accounts, and the one shown, survive restarts (M4-01).
        val accountsModel: AccountsViewModel = hiltViewModel()
        val accounts by accountsModel.state.collectAsStateWithLifecycle()
        var addingAccount by rememberSaveable { mutableStateOf(false) }
        // A widget tap opens that message's account first.
        LaunchedEffect(openRequest, accounts.accounts) {
            openRequest?.let { (account, _) ->
                if (accounts.currentId !=
                    account
                ) {
                    accountsModel.select(account)
                }
            }
        }
        val id = accounts.currentId
        NotificationPermissionPrompt(hasAccount = id != null)
        when {
            addingAccount -> {
                BackHandler { addingAccount = false }
                ManualSetupScreen(onAccountAdded = {
                    addingAccount = false
                    accountsModel.added(it)
                })
            }
            accounts.accounts == null -> Surface(color = MaterialTheme.colorScheme.surface) {}
            id != null ->
                // A new key per account, so its screens start fresh.
                key(id) {
                    MailHome(
                        accountId = id,
                        accounts = accounts.accounts.orEmpty(),
                        onSwitchAccount = accountsModel::select,
                        onAddAccount = { addingAccount = true },
                        onRemoveAccount = accountsModel::remove,
                        onAccountsChanged = accountsModel::refresh,
                        openMessageId = openRequest?.takeIf { it.first == id }?.second,
                        onMessageOpened = { openRequest = null },
                    )
                }
            else ->
                Surface(color = MaterialTheme.colorScheme.surface) {
                    WelcomeState(onAddAccount = { addingAccount = true })
                }
        }
    }

    /**
     * Asks once for the notification permission (Android 13+) once there is an account, so new mail
     * can be notified (M4-05). A refusal is remembered; Settings can ask again.
     */
    @Composable
    private fun NotificationPermissionPrompt(hasAccount: Boolean) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val launcher =
            rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
                lifecycleScope.launch { settings.setNotificationPromptAnswered() }
            }
        LaunchedEffect(hasAccount) {
            if (!hasAccount) return@LaunchedEffect
            val granted =
                ContextCompat.checkSelfPermission(this@MainActivity, Manifest.permission.POST_NOTIFICATIONS) ==
                    PackageManager.PERMISSION_GRANTED
            if (!granted && !settings.notificationPromptAnswered.first() && settings.notificationsEnabled.first()) {
                launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
    }
}

private const val SNOOZE_CHECK_MS = 60_000L
