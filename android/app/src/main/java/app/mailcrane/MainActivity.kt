package app.mailcrane

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import app.mailcrane.ui.HelloScreen
import app.mailcrane.ui.theme.MailcraneTheme
import dagger.hilt.android.AndroidEntryPoint

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MailcraneTheme {
                HelloScreen()
            }
        }
    }
}
