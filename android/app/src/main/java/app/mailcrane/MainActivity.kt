package app.mailcrane

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import app.mailcrane.ui.HelloScreen
import app.mailcrane.ui.account.ManualSetupScreen
import app.mailcrane.ui.theme.MailcraneTheme
import dagger.hilt.android.AndroidEntryPoint

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MailcraneTheme {
                // Until the inbox exists (M1-05 onwards), adding an account leads back to the
                // greeting screen.
                var accountAdded by rememberSaveable { mutableStateOf(false) }
                if (accountAdded) {
                    HelloScreen()
                } else {
                    ManualSetupScreen(onAccountAdded = { accountAdded = true })
                }
            }
        }
    }
}
