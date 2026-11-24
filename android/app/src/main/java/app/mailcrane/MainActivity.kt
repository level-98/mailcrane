package app.mailcrane

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import app.mailcrane.ui.account.ManualSetupScreen
import app.mailcrane.ui.components.WelcomeState
import app.mailcrane.ui.folders.MailboxScreen
import app.mailcrane.ui.theme.MailcraneTheme
import dagger.hilt.android.AndroidEntryPoint

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MailcraneTheme {
                // Adding an account opens its mail. Remembering accounts across restarts comes
                // with the account list; until then, no account id means no accounts.
                var accountId by rememberSaveable { mutableStateOf<Long?>(null) }
                var addingAccount by rememberSaveable { mutableStateOf(false) }
                val id = accountId
                when {
                    id != null -> MailboxScreen(accountId = id)
                    addingAccount -> {
                        BackHandler { addingAccount = false }
                        ManualSetupScreen(onAccountAdded = { accountId = it })
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
