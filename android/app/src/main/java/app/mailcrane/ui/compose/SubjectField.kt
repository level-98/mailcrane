package app.mailcrane.ui.compose

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import app.mailcrane.R

/**
 * The subject, with a text of its own (#429). `state.subject` moves on with every edit the field sends, but often a
 * frame or two behind the typing; fed straight back into a `String` field, an older subject reset the text and the
 * cursor, and characters were lost. Only a new [ComposeUiState.subjectRevision] (a prefill, draft, reply or reset)
 * replaces the field's text, the same approach as the body field (#419).
 */
@Composable
internal fun SubjectField(
    state: ComposeUiState,
    /** The field's text, and the [ComposeUiState.subjectRevision] it was based on. */
    onSubjectChange: (String, Long) -> Unit,
    enabled: Boolean,
    colors: TextFieldColors,
    /** The view model's state right now, which can be a revision ahead of [state]. */
    currentState: (() -> ComposeUiState)? = null,
) {
    val composed by rememberUpdatedState(state)
    var value by remember { mutableStateOf(TextFieldValue(state.subject, TextRange(state.subject.length))) }
    var appliedRevision by remember { mutableLongStateOf(state.subjectRevision) }
    LaunchedEffect(state.subjectRevision) {
        // The newest revision there is, not the one this effect was launched for.
        val now = currentState?.invoke()?.takeIf { it.subjectRevision >= composed.subjectRevision } ?: composed
        if (now.subjectRevision <= appliedRevision) return@LaunchedEffect
        appliedRevision = now.subjectRevision
        val incoming = now.subject
        // A subject loaded into an empty field puts the cursor at its end; otherwise it keeps its place.
        val selection = if (value.text.isEmpty()) TextRange(incoming.length) else value.selection
        value =
            TextFieldValue(
                text = incoming,
                selection =
                    TextRange(
                        selection.start.coerceIn(0, incoming.length),
                        selection.end.coerceIn(0, incoming.length),
                    ),
            )
    }
    TextField(
        value = value,
        onValueChange = { new ->
            val before = value.text
            value = new
            // Compared with the field's own previous text, not with state.subject, which may be behind.
            if (new.text != before) onSubjectChange(new.text, appliedRevision)
        },
        enabled = enabled,
        singleLine = true,
        placeholder = { Text(stringResource(R.string.compose_subject)) },
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        colors = colors,
        modifier = Modifier.fillMaxWidth().testTag("compose-subject"),
    )
}
