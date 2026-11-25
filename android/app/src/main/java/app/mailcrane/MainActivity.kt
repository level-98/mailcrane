package app.mailcrane

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import app.mailcrane.settings.ThemeMode
import app.mailcrane.ui.MailHome
import app.mailcrane.ui.account.AccountsViewModel
import app.mailcrane.ui.account.ManualSetupScreen
import app.mailcrane.ui.components.WelcomeState
import app.mailcrane.ui.theme.LocalDarkMessageBodies
import app.mailcrane.ui.theme.LocalDarkTheme
import app.mailcrane.ui.theme.MailcraneTheme
import app.mailcrane.ui.theme.ThemeViewModel
import app.mailcrane.work.SnoozeWaker
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import javax.inject.Inject

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    @Inject lateinit var snoozeWaker: SnoozeWaker

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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
            val dark =
                when (appearance.mode) {
                    ThemeMode.SYSTEM -> isSystemInDarkTheme()
                    ThemeMode.LIGHT -> false
                    ThemeMode.DARK -> true
                }
            MailcraneTheme(darkTheme = dark, dynamicColor = appearance.dynamicColor) {
                CompositionLocalProvider(
                    LocalDarkTheme provides dark,
                    LocalDarkMessageBodies provides appearance.darkMessageBodies,
                ) {
                    // The saved accounts, and the one shown, survive restarts (M4-01).
                    val accountsModel: AccountsViewModel = hiltViewModel()
                    val accounts by accountsModel.state.collectAsStateWithLifecycle()
                    var addingAccount by rememberSaveable { mutableStateOf(false) }
                    val id = accounts.currentId
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
                                )
                            }
                        else ->
                            Surface(color = MaterialTheme.colorScheme.surface) {
                                WelcomeState(onAddAccount = { addingAccount = true })
                            }
                    }
                }
            }
        }
    }
}

private const val SNOOZE_CHECK_MS = 60_000L
