package app.mailcrane.ui.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.contacts.RecipientSuggestion

/** Suggested recipients under the field being typed in (M2-08); tapping one adds it as a chip. */
@Composable
internal fun RecipientSuggestions(
    suggestions: List<RecipientSuggestion>,
    onPick: (RecipientSuggestion) -> Unit,
) {
    Surface(
        tonalElevation = 2.dp,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp).testTag("recipient-suggestions"),
    ) {
        Column {
            suggestions.forEach { s ->
                ListItem(
                    headlineContent = { Text(s.name.ifBlank { s.email }) },
                    supportingContent = if (s.name.isBlank()) null else ({ Text(s.email) }),
                    modifier = Modifier.clickable { onPick(s) },
                )
            }
        }
    }
}

/** "Show suggestions from your contacts?", asked once, the first time a recipient is typed. */
@Composable
internal fun ContactsPrompt(
    onAllow: () -> Unit,
    onDecline: () -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp).testTag("contacts-prompt"),
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 4.dp)) {
            Text(stringResource(R.string.contacts_prompt_title), style = MaterialTheme.typography.bodyMedium)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = onDecline) { Text(stringResource(R.string.contacts_prompt_no)) }
                TextButton(onClick = onAllow) { Text(stringResource(R.string.contacts_prompt_yes)) }
            }
        }
    }
}
