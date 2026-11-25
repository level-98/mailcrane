package app.mailcrane.ui.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenu
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.RecipientField

/** Writes a new message from account [accountId]. [onDone] is called once it is sent or closed. */
@Composable
fun ComposeScreen(
    accountId: Long,
    onDone: (ComposeResult) -> Unit,
    modifier: Modifier = Modifier,
    reply: ReplyRequest? = null,
    draft: DraftRequest? = null,
    viewModel: ComposeViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel, accountId, reply, draft) { viewModel.start(accountId, reply, draft) }
    LaunchedEffect(state.result) {
        state.result?.let {
            viewModel.reset()
            onDone(it)
        }
    }
    BackHandler(onBack = viewModel::requestClose)
    ComposeContent(
        state = state,
        actions =
            ComposeActions(
                onFromChange = viewModel::onFromChange,
                onRecipientInput = viewModel::onRecipientInput,
                onCommitRecipient = viewModel::commitRecipient,
                onRemoveRecipient = viewModel::removeRecipient,
                onEditLastRecipient = viewModel::editLastRecipient,
                onShowCcBcc = viewModel::showCcBcc,
                onSubjectChange = viewModel::onSubjectChange,
                onBodyChange = viewModel::onBodyChange,
                onSend = viewModel::send,
                onSchedule = viewModel::schedule,
                onClose = viewModel::requestClose,
                onDiscard = viewModel::requestDiscard,
                onConfirmDiscard = viewModel::confirmDiscard,
                onKeepEditing = viewModel::keepEditing,
                onDismissError = viewModel::dismissError,
            ),
        modifier = modifier,
    )
}

/** What the compose screen can ask its view model to do. */
data class ComposeActions(
    val onFromChange: (Long) -> Unit = {},
    val onRecipientInput: (RecipientField, String) -> Unit = { _, _ -> },
    val onCommitRecipient: (RecipientField) -> Unit = {},
    val onRemoveRecipient: (RecipientField, Int) -> Unit = { _, _ -> },
    val onEditLastRecipient: (RecipientField) -> Unit = {},
    val onShowCcBcc: () -> Unit = {},
    val onSubjectChange: (String) -> Unit = {},
    val onBodyChange: (String) -> Unit = {},
    val onSend: () -> Unit = {},
    val onSchedule: (Long) -> Unit = {},
    val onClose: () -> Unit = {},
    val onDiscard: () -> Unit = {},
    val onConfirmDiscard: () -> Unit = {},
    val onKeepEditing: () -> Unit = {},
    val onDismissError: () -> Unit = {},
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ComposeContent(
    state: ComposeUiState,
    actions: ComposeActions,
    modifier: Modifier = Modifier,
) {
    Scaffold(
        modifier = modifier,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.compose_title)) },
                navigationIcon = {
                    IconButton(onClick = actions.onClose, enabled = !state.sending) {
                        Icon(
                            painterResource(R.drawable.ic_mc_close),
                            contentDescription = stringResource(R.string.compose_close),
                        )
                    }
                },
                actions = {
                    IconButton(onClick = actions.onDiscard, enabled = !state.sending) {
                        Icon(
                            painter = painterResource(R.drawable.ic_mc_trash),
                            contentDescription = stringResource(R.string.compose_discard),
                        )
                    }
                    ScheduleButton(enabled = state.canSend, onSchedule = actions.onSchedule)
                    SendButton(state, actions.onSend)
                },
            )
        },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .consumeWindowInsets(padding)
                    .imePadding()
                    .verticalScroll(rememberScrollState()),
        ) {
            state.error?.let { ErrorBanner(it, state.errorAddresses, actions.onDismissError) }
            val editable = !state.sending
            if (state.showFromPicker) {
                FromPicker(state.accounts, state.fromAccountId, editable, actions.onFromChange)
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            }
            Recipient(RecipientField.TO, R.string.compose_to, state, actions, editable) {
                if (!state.showCcBcc) {
                    TextButton(onClick = actions.onShowCcBcc, enabled = editable) {
                        Text(stringResource(R.string.compose_cc_bcc))
                    }
                }
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            if (state.showCcBcc) {
                Recipient(RecipientField.CC, R.string.compose_cc, state, actions, editable)
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Recipient(RecipientField.BCC, R.string.compose_bcc, state, actions, editable)
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            }
            TextField(
                value = state.subject,
                onValueChange = actions.onSubjectChange,
                enabled = editable,
                singleLine = true,
                placeholder = { Text(stringResource(R.string.compose_subject)) },
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                colors = plainFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            TextField(
                value = state.body,
                onValueChange = actions.onBodyChange,
                enabled = editable,
                minLines = 8,
                placeholder = { Text(stringResource(R.string.compose_body)) },
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                colors = plainFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
    if (state.confirmingDiscard) {
        AlertDialog(
            onDismissRequest = actions.onKeepEditing,
            title = { Text(stringResource(R.string.compose_discard_title)) },
            text = { Text(stringResource(R.string.compose_discard_text)) },
            confirmButton = {
                TextButton(
                    onClick = actions.onConfirmDiscard,
                ) { Text(stringResource(R.string.compose_discard_confirm)) }
            },
            dismissButton = {
                TextButton(onClick = actions.onKeepEditing) { Text(stringResource(R.string.compose_discard_keep)) }
            },
        )
    }
}

@Composable
private fun SendButton(
    state: ComposeUiState,
    onSend: () -> Unit,
) {
    val sendLabel = stringResource(R.string.compose_send)
    Button(
        onClick = onSend,
        enabled = state.canSend,
        contentPadding = ButtonDefaults.ButtonWithIconContentPadding,
        modifier = Modifier.padding(end = 8.dp),
    ) {
        if (state.sending) {
            CircularProgressIndicator(modifier = Modifier.size(ButtonDefaults.IconSize), strokeWidth = 2.dp)
            Text(
                stringResource(R.string.compose_sending),
                modifier = Modifier.padding(start = ButtonDefaults.IconSpacing),
            )
        } else {
            Icon(
                painterResource(R.drawable.ic_mc_sent),
                contentDescription = null,
                modifier = Modifier.size(ButtonDefaults.IconSize),
            )
            Text(sendLabel, modifier = Modifier.padding(start = ButtonDefaults.IconSpacing))
        }
    }
}

@Composable
private fun Recipient(
    which: RecipientField,
    label: Int,
    state: ComposeUiState,
    actions: ComposeActions,
    enabled: Boolean,
    trailing: (@Composable () -> Unit)? = null,
) {
    RecipientField(
        label = stringResource(label),
        state = state.field(which),
        onInput = { actions.onRecipientInput(which, it) },
        onCommit = { actions.onCommitRecipient(which) },
        onRemove = { actions.onRemoveRecipient(which, it) },
        onBackspaceWhenEmpty = { actions.onEditLastRecipient(which) },
        enabled = enabled,
        trailing = trailing,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun FromPicker(
    accounts: List<AccountSummary>,
    selectedId: Long?,
    enabled: Boolean,
    onSelect: (Long) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    val selected = accounts.firstOrNull { it.id == selectedId }
    ExposedDropdownMenuBox(
        expanded = expanded,
        onExpandedChange = { if (enabled) expanded = it },
    ) {
        TextField(
            value = selected?.let(::accountLabel).orEmpty(),
            onValueChange = {},
            readOnly = true,
            enabled = enabled,
            // Wraps instead of cutting the address off at large text sizes.
            maxLines = 3,
            label = { Text(stringResource(R.string.compose_from)) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) },
            colors = plainFieldColors(),
            modifier = Modifier.fillMaxWidth().menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable, enabled),
        )
        ExposedDropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            accounts.forEach { account ->
                DropdownMenuItem(
                    text = { Text(accountLabel(account)) },
                    onClick = {
                        onSelect(account.id)
                        expanded = false
                    },
                )
            }
        }
    }
}

private fun accountLabel(account: AccountSummary): String =
    if (account.displayName.isBlank()) account.email else "${account.displayName} <${account.email}>"

@Composable
private fun ErrorBanner(
    error: ComposeError,
    addresses: List<String>,
    onDismiss: () -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.errorContainer,
        contentColor = MaterialTheme.colorScheme.onErrorContainer,
        modifier = Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Row(
            modifier = Modifier.padding(start = 16.dp, top = 12.dp, bottom = 12.dp, end = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                painterResource(R.drawable.ic_mc_error),
                contentDescription = null,
                modifier = Modifier.padding(top = 2.dp),
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(errorMessage(error, addresses), style = MaterialTheme.typography.bodyMedium)
                Text(stringResource(R.string.compose_error_kept), style = MaterialTheme.typography.bodySmall)
            }
            IconButton(onClick = onDismiss) {
                Icon(
                    painterResource(R.drawable.ic_mc_close),
                    contentDescription = stringResource(R.string.compose_error_dismiss),
                )
            }
        }
    }
}

@Composable
private fun errorMessage(
    error: ComposeError,
    addresses: List<String>,
): String =
    when (error) {
        ComposeError.ALL_REJECTED -> stringResource(R.string.compose_error_all_rejected, addresses.joinToString(", "))
        else -> stringResource(errorText(error))
    }

internal fun errorText(error: ComposeError): Int =
    when (error) {
        ComposeError.INVALID_RECIPIENTS -> R.string.compose_error_invalid_recipients
        ComposeError.NO_RECIPIENTS -> R.string.compose_error_no_recipients
        ComposeError.TOO_MANY_RECIPIENTS -> R.string.compose_error_too_many_recipients
        ComposeError.TOO_LARGE -> R.string.compose_error_too_large
        ComposeError.ACCOUNT_NOT_FOUND -> R.string.compose_error_account_not_found
        ComposeError.INVALID_SENDER -> R.string.compose_error_invalid_sender
        ComposeError.MISSING_CREDENTIALS -> R.string.compose_error_missing_credentials
        ComposeError.STORAGE -> R.string.compose_error_storage
        ComposeError.CANNOT_CONNECT -> R.string.compose_error_cannot_connect
        ComposeError.TLS -> R.string.compose_error_tls
        ComposeError.STARTTLS_NOT_OFFERED -> R.string.compose_error_starttls
        ComposeError.AUTH -> R.string.compose_error_auth
        ComposeError.TIMEOUT -> R.string.compose_error_timeout
        ComposeError.ALL_REJECTED -> R.string.compose_error_all_rejected
        ComposeError.REFUSED_TEMPORARY -> R.string.compose_error_refused_temporary
        ComposeError.REFUSED_PERMANENT -> R.string.compose_error_refused_permanent
        ComposeError.UNEXPECTED -> R.string.compose_error_unexpected
        ComposeError.REPLY_UNAVAILABLE -> R.string.compose_error_reply_unavailable
        ComposeError.DRAFT_UNAVAILABLE -> R.string.compose_error_draft_unavailable
    }

@Composable
private fun plainFieldColors() =
    TextFieldDefaults.colors(
        focusedContainerColor = Color.Transparent,
        unfocusedContainerColor = Color.Transparent,
        disabledContainerColor = Color.Transparent,
        focusedIndicatorColor = Color.Transparent,
        unfocusedIndicatorColor = Color.Transparent,
        disabledIndicatorColor = Color.Transparent,
    )
