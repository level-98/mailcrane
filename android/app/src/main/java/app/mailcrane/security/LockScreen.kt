package app.mailcrane.security

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import app.mailcrane.R

/**
 * Covers the whole app while it's locked (M5-03). Opens the system prompt by itself each time the
 * screen comes to the front; the button opens it again after a cancel.
 */
@Composable
fun LockScreen(appLock: AppLock) {
    val context = LocalContext.current
    val activity = context as? FragmentActivity
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val title = stringResource(R.string.app_lock_prompt_title)
    var prompting by remember { mutableStateOf(false) }
    val canUnlock = remember(context) { DeviceAuth.available(context) }

    fun unlock() {
        if (activity == null || prompting || !canUnlock) return
        prompting = true
        appLock.beginAuthentication()
        DeviceAuth.prompt(activity, title) { ok ->
            prompting = false
            appLock.endAuthentication(ok)
        }
    }

    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) { unlock() }
    }

    Surface(color = MaterialTheme.colorScheme.surface, modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier.fillMaxSize().padding(32.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Icon(
                painter = painterResource(R.drawable.ic_mc_shield),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(56.dp),
            )
            Spacer(Modifier.height(24.dp))
            Text(
                text = stringResource(R.string.app_lock_locked_title),
                style = MaterialTheme.typography.headlineSmall,
                textAlign = TextAlign.Center,
                modifier = Modifier.semantics { heading() },
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text =
                    stringResource(
                        if (canUnlock) R.string.app_lock_locked_body else R.string.app_lock_no_credential_locked,
                    ),
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            if (canUnlock) {
                Spacer(Modifier.height(32.dp))
                Button(onClick = { unlock() }) { Text(stringResource(R.string.app_lock_unlock)) }
            }
        }
    }
}
