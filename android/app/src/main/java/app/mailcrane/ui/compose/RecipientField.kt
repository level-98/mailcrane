package app.mailcrane.ui.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.InputChip
import androidx.compose.material3.InputChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.mailcrane.R

/**
 * A To, Cc or Bcc field. Finished addresses show as chips; invalid ones use the error colour and an
 * error icon, so the state never relies on colour alone.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RecipientField(
    label: String,
    state: RecipientFieldState,
    onInput: (String) -> Unit,
    onCommit: () -> Unit,
    onRemove: (Int) -> Unit,
    onBackspaceWhenEmpty: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    trailing: (@Composable () -> Unit)? = null,
) {
    val focusManager = LocalFocusManager.current
    var focused by remember { mutableStateOf(false) }
    Row(
        modifier = modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.widthIn(min = 48.dp).padding(end = 8.dp),
        )
        FlowRow(
            modifier = Modifier.weight(1f),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
            itemVerticalAlignment = Alignment.CenterVertically,
        ) {
            state.chips.forEachIndexed { index, chip ->
                RecipientChipView(chip = chip, enabled = enabled, onRemove = { onRemove(index) })
            }
            BasicTextField(
                value = state.input,
                onValueChange = onInput,
                enabled = enabled,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                keyboardOptions =
                    KeyboardOptions(
                        keyboardType = KeyboardType.Email,
                        imeAction = ImeAction.Next,
                        autoCorrectEnabled = false,
                    ),
                keyboardActions =
                    KeyboardActions(onNext = {
                        onCommit()
                        focusManager.moveFocus(FocusDirection.Down)
                    }),
                modifier =
                    Modifier
                        .weight(1f)
                        .widthIn(min = 96.dp)
                        .padding(vertical = 10.dp)
                        .semantics { contentDescription = label }
                        .onFocusChanged {
                            // Leaving the field finishes what was typed.
                            if (focused && !it.isFocused) onCommit()
                            focused = it.isFocused
                        }.onPreviewKeyEvent {
                            val backspace = it.type == KeyEventType.KeyDown && it.key == Key.Backspace
                            if (backspace && state.input.isEmpty() && state.chips.isNotEmpty()) {
                                onBackspaceWhenEmpty()
                                true
                            } else {
                                false
                            }
                        },
            )
        }
        trailing?.invoke()
    }
}

@Composable
private fun RecipientChipView(
    chip: RecipientChip,
    enabled: Boolean,
    onRemove: () -> Unit,
) {
    val invalid = !chip.isValid
    val description =
        if (invalid) {
            stringResource(R.string.compose_recipient_invalid, chip.label)
        } else {
            chip.email.takeIf { chip.name.isNotBlank() }?.let { "${chip.name}, $it" } ?: chip.label
        }
    val removeLabel = stringResource(R.string.compose_remove_recipient, chip.label)
    InputChip(
        selected = false,
        onClick = {},
        enabled = enabled,
        label = { Text(chip.label, maxLines = 1) },
        leadingIcon =
            if (invalid) {
                {
                    Icon(
                        painter = painterResource(R.drawable.ic_mc_error),
                        contentDescription = null,
                        // onErrorContainer keeps contrast on the chip in light and dark themes.
                        tint = MaterialTheme.colorScheme.onErrorContainer,
                        modifier = Modifier.size(InputChipDefaults.IconSize),
                    )
                }
            } else {
                null
            },
        trailingIcon = {
            Icon(
                painter = painterResource(R.drawable.ic_mc_close),
                contentDescription = removeLabel,
                modifier =
                    Modifier
                        .size(InputChipDefaults.IconSize)
                        .clickable(
                            enabled = enabled,
                            role = Role.Button,
                            onClickLabel = removeLabel,
                            onClick = onRemove,
                        ),
            )
        },
        colors =
            if (invalid) {
                InputChipDefaults.inputChipColors(
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                    labelColor = MaterialTheme.colorScheme.onErrorContainer,
                    trailingIconColor = MaterialTheme.colorScheme.onErrorContainer,
                )
            } else {
                InputChipDefaults.inputChipColors()
            },
        border =
            if (invalid) {
                InputChipDefaults.inputChipBorder(
                    enabled = enabled,
                    selected = false,
                    borderColor = MaterialTheme.colorScheme.error,
                )
            } else {
                InputChipDefaults.inputChipBorder(enabled = enabled, selected = false)
            },
        modifier = Modifier.semantics { contentDescription = description },
    )
}
