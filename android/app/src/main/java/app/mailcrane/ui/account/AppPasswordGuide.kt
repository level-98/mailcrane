package app.mailcrane.ui.account

import androidx.annotation.ArrayRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringArrayResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.AppPasswordGuide
import app.mailcrane.core.AppPasswordProvider

/**
 * How to make an app password for [guide]'s provider (M3-05): short numbered steps from the strings
 * files, and a button that opens the provider's own page in the system browser. The app makes no
 * request of its own; signing in then goes through the usual check.
 */
@Composable
internal fun AppPasswordGuideCard(guide: AppPasswordGuide) {
    val uriHandler = LocalUriHandler.current
    Card(Modifier.fillMaxWidth().testTag("app-password-guide")) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                text = stringResource(R.string.app_password_title, guide.providerName),
                style = MaterialTheme.typography.titleSmall,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                text = stringResource(R.string.app_password_intro, guide.providerName),
                style = MaterialTheme.typography.bodyMedium,
            )
            stringArrayResource(guide.provider.steps()).forEachIndexed { i, step ->
                Row {
                    Text(
                        text = stringResource(R.string.app_password_step_number, i + 1),
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.width(24.dp),
                    )
                    Text(text = step, style = MaterialTheme.typography.bodyMedium)
                }
            }
            OutlinedButton(
                onClick = { uriHandler.openUri(guide.url) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(stringResource(R.string.app_password_open, guide.providerName))
            }
        }
    }
}

@ArrayRes
private fun AppPasswordProvider.steps(): Int =
    when (this) {
        AppPasswordProvider.ICLOUD -> R.array.app_password_steps_icloud
        AppPasswordProvider.YAHOO -> R.array.app_password_steps_yahoo
        AppPasswordProvider.AOL -> R.array.app_password_steps_aol
    }
