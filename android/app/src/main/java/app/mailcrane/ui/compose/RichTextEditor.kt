package app.mailcrane.ui.compose

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import app.mailcrane.R

/** What the rich editor's toolbar does (M2-09). */
data class RichTextActions(
    val onFormat: (Int, Int, InlineFormat) -> Unit = { _, _, _ -> },
    val onLink: (Int, Int, String?) -> Unit = { _, _, _ -> },
    /** Returns the new selection. */
    val onList: (Int, Int, ListKind) -> Pair<Int, Int> = { s, e, _ -> s to e },
    val onPlainTextOnly: (Boolean) -> Unit = {},
)

/**
 * The message body with a formatting toolbar: Bold, Italic, bulleted and numbered lists, Link, and
 * "Plain text" to send without formatting. The text itself stays plain; styles are drawn on top.
 */
@Composable
internal fun RichBodyField(
    state: ComposeUiState,
    onBodyChange: (String) -> Unit,
    actions: RichTextActions,
    enabled: Boolean,
    colors: androidx.compose.material3.TextFieldColors,
) {
    var value by remember {
        mutableStateOf(TextFieldValue(state.body, TextRange(if (state.bodyCursorAtEnd) state.body.length else 0)))
    }
    // Texts this field has sent with onBodyChange that haven't come back in state.body yet, oldest
    // first (#419). Typing can outrun the view model: state.body then shows an older edit of this
    // field's own, which must not be put back into it. Not state: changing it redraws nothing.
    val inFlight = remember { ArrayDeque<String>() }
    // The selection a list toggle asked for, applied once its new text arrives.
    var selectionForNextText by remember { mutableStateOf<TextRange?>(null) }
    LaunchedEffect(state.body) {
        val incoming = state.body
        val echo = inFlight.indexOf(incoming)
        if (echo >= 0) {
            // This field's own edit, back late: everything sent up to it has arrived.
            repeat(echo + 1) { inFlight.removeFirst() }
        } else {
            // Text set from outside (reply, draft, signature, list toggles) replaces the field's
            // text. Whatever was still in flight has been overtaken.
            inFlight.clear()
            if (value.text != incoming) {
                val asked = selectionForNextText
                val selection =
                    when {
                        asked != null -> asked
                        value.text.isEmpty() && state.bodyCursorAtEnd -> TextRange(incoming.length)
                        else -> value.selection
                    }
                value =
                    value.copy(
                        text = incoming,
                        selection =
                            TextRange(
                                selection.start.coerceAtMost(incoming.length),
                                selection.end.coerceAtMost(incoming.length),
                            ),
                    )
            }
        }
        selectionForNextText = null
    }
    var askingLink by remember { mutableStateOf(false) }
    val linkColor = MaterialTheme.colorScheme.primary
    val styles = state.bodyStyles
    val sel = value.selection
    if (!state.plainTextOnly) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .horizontalScroll(
                        rememberScrollState(),
                    ).padding(horizontal = 8.dp)
                    .testTag("rich-toolbar"),
        ) {
            ToolbarButton("B", R.string.rich_bold, enabled, weight = FontWeight.Bold) {
                actions.onFormat(sel.min, sel.max, InlineFormat.BOLD)
            }
            ToolbarButton("I", R.string.rich_italic, enabled, italic = true) {
                actions.onFormat(sel.min, sel.max, InlineFormat.ITALIC)
            }
            // The new selection refers to the toggled text, which arrives through state.body; set
            // now, it would be cut down to the old text's length.
            ToolbarButton("•", R.string.rich_bullets, enabled) {
                val (s, e) = actions.onList(sel.min, sel.max, ListKind.BULLET)
                selectionForNextText = TextRange(s, e)
            }
            ToolbarButton("1.", R.string.rich_numbers, enabled) {
                val (s, e) = actions.onList(sel.min, sel.max, ListKind.NUMBERED)
                selectionForNextText = TextRange(s, e)
            }
            ToolbarButton("🔗", R.string.rich_link, enabled && !sel.collapsed) { askingLink = true }
        }
    }
    Row(Modifier.padding(horizontal = 16.dp)) {
        FilterChip(
            selected = state.plainTextOnly,
            onClick = { actions.onPlainTextOnly(!state.plainTextOnly) },
            label = { Text(stringResource(R.string.rich_plain_only)) },
            enabled = enabled,
        )
    }
    TextField(
        value = value,
        onValueChange = { new ->
            // Typing after a list toggle that changed nothing: its selection is no longer wanted.
            selectionForNextText = null
            // Enter at the end of a list line starts the next item.
            val continued = continueList(value.text, new.text, new.selection.end)
            value =
                if (continued !=
                    null
                ) {
                    new.copy(text = continued.first, selection = TextRange(continued.second))
                } else {
                    new
                }
            // Compared with the last text sent, not with state.body, which may be an older edit:
            // deleting back to it while it is stale must still be sent.
            if (value.text != (inFlight.lastOrNull() ?: state.body)) {
                inFlight.addLast(value.text)
                onBodyChange(value.text)
            }
        },
        enabled = enabled,
        minLines = 8,
        placeholder = { Text(stringResource(R.string.compose_body)) },
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        visualTransformation =
            if (state.plainTextOnly) {
                VisualTransformation.None
            } else {
                VisualTransformation { text ->
                    TransformedText(styledText(text.text, styles, linkColor), OffsetMapping.Identity)
                }
            },
        colors = colors,
        modifier = Modifier.fillMaxWidth().testTag("compose-body"),
    )
    if (askingLink) {
        LinkDialog(
            initial = fitStyles(state.body, styles).getOrNull(sel.min)?.link.orEmpty(),
            onDone = { url ->
                askingLink = false
                actions.onLink(sel.min, sel.max, url)
            },
            onDismiss = { askingLink = false },
        )
    }
}

@Composable
private fun ToolbarButton(
    label: String,
    description: Int,
    enabled: Boolean,
    weight: FontWeight? = null,
    italic: Boolean = false,
    onClick: () -> Unit,
) {
    val spoken = stringResource(description)
    TextButton(onClick = onClick, enabled = enabled, modifier = Modifier.semantics { contentDescription = spoken }) {
        Text(label, fontWeight = weight, fontStyle = if (italic) FontStyle.Italic else null)
    }
}

/** Asks for the link's address; an empty one removes the link. Only https, http and mailto are sent. */
@Composable
private fun LinkDialog(
    initial: String,
    onDone: (String?) -> Unit,
    onDismiss: () -> Unit,
) {
    var url by remember { mutableStateOf(initial.ifEmpty { "https://" }) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.rich_link_title)) },
        text = {
            OutlinedTextField(
                value = url,
                onValueChange = { url = it },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                label = { Text(stringResource(R.string.rich_link_address)) },
            )
        },
        confirmButton = {
            TextButton(onClick = {
                onDone(url.trim().takeIf { it.isNotEmpty() && it != "https://" })
            }) { Text(stringResource(R.string.rich_link_ok)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.rich_link_cancel)) } },
    )
}
