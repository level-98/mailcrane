package app.mailcrane

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import app.mailcrane.ui.account.ManualSetupScreen
import app.mailcrane.ui.messagelist.InboxScreen
import app.mailcrane.ui.theme.MailcraneTheme
import dagger.hilt.android.AndroidEntryPoint

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MailcraneTheme {
                // Adding an account opens its inbox. Remembering accounts across restarts comes
                // with the account list.
                var accountId by rememberSaveable { mutableStateOf<Long?>(null) }
                val id = accountId
                if (id != null) {
                    InboxScreen(accountId = id)
                } else {
                    ManualSetupScreen(onAccountAdded = { accountId = it })
                }
            }
        }
    }
}
