package app.mailcrane.ui.account

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.Field
import app.mailcrane.core.FieldProblem
import app.mailcrane.core.MailServer
import app.mailcrane.oauth.OAuthResult
import app.mailcrane.oauth.OAuthSignIn

/** Add an IMAP/SMTP account by typing the server details. Calls [onAccountAdded] once it's saved. */
@Composable
fun ManualSetupScreen(
    onAccountAdded: (Long) -> Unit,
    viewModel: ManualSetupViewModel = hiltViewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    // Sign in with Google (M3-02): the browser's answer comes back through the redirect activity.
    LaunchedEffect(viewModel) { OAuthSignIn.results.collect(viewModel::onGoogleResult) }
    state.addedAccountId?.let { id ->
        LaunchedEffect(id) {
            onAccountAdded(id)
            viewModel.accountHandled()
        }
    }
    ManualSetupContent(
        state = state,
        actions =
            ManualSetupActions(
                onEmailChange = viewModel::onEmailChange,
                onDisplayNameChange = viewModel::onDisplayNameChange,
                onImapHostChange = viewModel::onImapHostChange,
                onImapPortChange = viewModel::onImapPortChange,
                onImapSecurityChange = viewModel::onImapSecurityChange,
                onSmtpHostChange = viewModel::onSmtpHostChange,
                onSmtpPortChange = viewModel::onSmtpPortChange,
                onSmtpSecurityChange = viewModel::onSmtpSecurityChange,
                onUsernameChange = viewModel::onUsernameChange,
                onPasswordChange = viewModel::onPasswordChange,
                onJmapChange = viewModel::onJmapChange,
                onJmapServerChange = viewModel::onJmapServerChange,
                onTogglePasswordVisible = viewModel::onTogglePasswordVisible,
                onSubmit = viewModel::submit,
                onDiscover = viewModel::discover,
                onEditSettings = viewModel::editSettings,
                onSetUpManually = viewModel::setUpManually,
                onSignInWithGoogle = {
                    viewModel.startGoogleSignIn()?.let { (provider, email) ->
                        if (!OAuthSignIn.start(context, provider, email)) {
                            viewModel.onGoogleResult(OAuthResult.Failed(null))
                        }
                    }
                },
            ),
    )
}

/** Everything the form can ask the view model to do. */
data class ManualSetupActions(
    val onEmailChange: (String) -> Unit = {},
    val onDisplayNameChange: (String) -> Unit = {},
    val onImapHostChange: (String) -> Unit = {},
    val onImapPortChange: (String) -> Unit = {},
    val onImapSecurityChange: (ConnectionSecurity) -> Unit = {},
    val onSmtpHostChange: (String) -> Unit = {},
    val onSmtpPortChange: (String) -> Unit = {},
    val onSmtpSecurityChange: (ConnectionSecurity) -> Unit = {},
    val onUsernameChange: (String) -> Unit = {},
    val onPasswordChange: (String) -> Unit = {},
    val onJmapChange: (Boolean) -> Unit = {},
    val onJmapServerChange: (String) -> Unit = {},
    val onTogglePasswordVisible: () -> Unit = {},
    val onSubmit: () -> Unit = {},
    val onDiscover: () -> Unit = {},
    val onEditSettings: () -> Unit = {},
    val onSetUpManually: () -> Unit = {},
    val onSignInWithGoogle: () -> Unit = {},
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ManualSetupContent(
    state: ManualSetupUiState,
    actions: ManualSetupActions,
) {
    when (state.step) {
        SetupStep.EMAIL -> EmailStepContent(state, actions)
        SetupStep.CONFIRM -> ConfirmStepContent(state, actions)
        SetupStep.MANUAL -> ManualFormContent(state, actions)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ManualFormContent(
    state: ManualSetupUiState,
    actions: ManualSetupActions,
) {
    val enabled = !state.checking
    Scaffold(
        topBar = { TopAppBar(title = { Text(stringResource(R.string.setup_title)) }) },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .imePadding()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            state.discoveryNote?.let { NoteCard(it.message()) }
            AccountTypeChoice(jmap = state.jmap, enabled = enabled, onChange = actions.onJmapChange)
            SectionTitle(R.string.setup_section_account)
            FormField(
                value = state.email,
                onValueChange = actions.onEmailChange,
                label = R.string.setup_email,
                error = state.fieldErrors[Field.EMAIL],
                enabled = enabled,
                keyboardType = KeyboardType.Email,
            )
            FormField(
                value = state.displayName,
                onValueChange = actions.onDisplayNameChange,
                label = R.string.setup_display_name,
                error = state.fieldErrors[Field.DISPLAY_NAME],
                enabled = enabled,
                capitalization = KeyboardCapitalization.Words,
            )

            if (state.jmap) {
                SectionTitle(R.string.setup_section_jmap)
                FormField(
                    value = state.jmapServer,
                    onValueChange = actions.onJmapServerChange,
                    label = R.string.setup_jmap_server,
                    error = state.fieldErrors[Field.IMAP_HOST],
                    enabled = enabled,
                    keyboardType = KeyboardType.Uri,
                )
                Text(
                    text = stringResource(R.string.setup_jmap_server_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                ImapSmtpSections(state, actions, enabled)
            }

            SectionTitle(R.string.setup_section_sign_in)
            FormField(
                value = state.username,
                onValueChange = actions.onUsernameChange,
                label = R.string.setup_username,
                error = state.fieldErrors[Field.USERNAME],
                enabled = enabled,
                keyboardType = KeyboardType.Email,
            )
            FormField(
                value = state.password,
                onValueChange = actions.onPasswordChange,
                label = R.string.setup_password,
                error = state.fieldErrors[Field.PASSWORD],
                enabled = enabled,
                keyboardType = KeyboardType.Password,
                imeAction = ImeAction.Done,
                visualTransformation =
                    if (state.passwordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                trailing = {
                    TextButton(onClick = actions.onTogglePasswordVisible) {
                        val label =
                            if (state.passwordVisible) R.string.setup_password_hide else R.string.setup_password_show
                        Text(stringResource(label))
                    }
                },
            )

            state.error?.let { ErrorCard(it) }

            Spacer(Modifier.height(8.dp))
            Button(
                onClick = actions.onSubmit,
                enabled = enabled,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (state.checking) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(18.dp),
                            strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.onPrimary,
                        )
                        Spacer(Modifier.size(12.dp))
                        Text(stringResource(R.string.setup_checking))
                    }
                } else {
                    Text(stringResource(R.string.setup_submit))
                }
            }
            Spacer(Modifier.height(16.dp))
        }
    }
}

@Composable
internal fun SectionTitle(
    @StringRes text: Int,
) {
    Text(
        text = stringResource(text),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(top = 12.dp),
    )
}

@Composable
internal fun FormField(
    value: String,
    onValueChange: (String) -> Unit,
    @StringRes label: Int,
    error: FieldProblem?,
    enabled: Boolean,
    modifier: Modifier = Modifier.fillMaxWidth(),
    keyboardType: KeyboardType = KeyboardType.Text,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    imeAction: ImeAction = ImeAction.Next,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    trailing: (@Composable () -> Unit)? = null,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(stringResource(label)) },
        isError = error != null,
        supportingText = error?.let { { Text(stringResource(it.message())) } },
        enabled = enabled,
        singleLine = true,
        keyboardOptions =
            KeyboardOptions(
                keyboardType = keyboardType,
                capitalization = capitalization,
                autoCorrectEnabled = false,
                imeAction = imeAction,
            ),
        visualTransformation = visualTransformation,
        trailingIcon = trailing,
        modifier = modifier,
    )
}

/** IMAP or JMAP, at the top of the manual form (M3-04). */
@Composable
private fun AccountTypeChoice(
    jmap: Boolean,
    enabled: Boolean,
    onChange: (Boolean) -> Unit,
) {
    SectionTitle(R.string.setup_section_type)
    Row(Modifier.selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilterChip(
            selected = !jmap,
            onClick = { onChange(false) },
            label = { Text(stringResource(R.string.setup_type_imap)) },
            enabled = enabled,
        )
        FilterChip(
            selected = jmap,
            onClick = { onChange(true) },
            label = { Text(stringResource(R.string.setup_type_jmap)) },
            enabled = enabled,
        )
    }
}

/** The incoming (IMAP) and outgoing (SMTP) server sections. */
@Composable
private fun ImapSmtpSections(
    state: ManualSetupUiState,
    actions: ManualSetupActions,
    enabled: Boolean,
) {
    SectionTitle(R.string.setup_section_imap)
    ServerFields(
        host = state.imapHost,
        port = state.imapPort,
        security = state.imapSecurity,
        hostError = state.fieldErrors[Field.IMAP_HOST],
        portError = state.fieldErrors[Field.IMAP_PORT],
        enabled = enabled,
        onHostChange = actions.onImapHostChange,
        onPortChange = actions.onImapPortChange,
        onSecurityChange = actions.onImapSecurityChange,
    )

    SectionTitle(R.string.setup_section_smtp)
    ServerFields(
        host = state.smtpHost,
        port = state.smtpPort,
        security = state.smtpSecurity,
        hostError = state.fieldErrors[Field.SMTP_HOST],
        portError = state.fieldErrors[Field.SMTP_PORT],
        enabled = enabled,
        onHostChange = actions.onSmtpHostChange,
        onPortChange = actions.onSmtpPortChange,
        onSecurityChange = actions.onSmtpSecurityChange,
    )
}

@Composable
private fun ServerFields(
    host: String,
    port: String,
    security: ConnectionSecurity,
    hostError: FieldProblem?,
    portError: FieldProblem?,
    enabled: Boolean,
    onHostChange: (String) -> Unit,
    onPortChange: (String) -> Unit,
    onSecurityChange: (ConnectionSecurity) -> Unit,
) {
    FormField(
        value = host,
        onValueChange = onHostChange,
        label = R.string.setup_host,
        error = hostError,
        enabled = enabled,
        keyboardType = KeyboardType.Uri,
    )
    SecurityDropdown(security = security, enabled = enabled, onChange = onSecurityChange)
    FormField(
        value = port,
        onValueChange = onPortChange,
        label = R.string.setup_port,
        error = portError,
        enabled = enabled,
        keyboardType = KeyboardType.Number,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SecurityDropdown(
    security: ConnectionSecurity,
    enabled: Boolean,
    onChange: (ConnectionSecurity) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    ExposedDropdownMenuBox(
        expanded = expanded,
        onExpandedChange = { if (enabled) expanded = it },
    ) {
        OutlinedTextField(
            value = stringResource(security.label()),
            onValueChange = {},
            readOnly = true,
            enabled = enabled,
            singleLine = true,
            label = { Text(stringResource(R.string.setup_security)) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) },
            modifier =
                Modifier
                    .fillMaxWidth()
                    .menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable, enabled),
        )
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false },
            modifier = Modifier.exposedDropdownSize(),
        ) {
            // Only encrypted choices exist; there is no plain-text option to offer.
            ConnectionSecurity.entries.forEach { option ->
                DropdownMenuItem(
                    text = { Text(stringResource(option.label())) },
                    onClick = {
                        expanded = false
                        onChange(option)
                    },
                )
            }
        }
    }
}

/** A neutral note above the form, such as "no settings were found". */
@Composable
internal fun NoteCard(
    @StringRes text: Int,
) {
    Card(
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.secondaryContainer,
                contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
            ),
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(top = 8.dp)
                .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Text(
            text = stringResource(text),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(16.dp),
        )
    }
}

@StringRes
private fun DiscoveryNote.message(): Int =
    when (this) {
        DiscoveryNote.NOT_FOUND -> R.string.setup_auto_not_found
        DiscoveryNote.TIMED_OUT -> R.string.setup_auto_timed_out
    }

@Composable
internal fun ErrorCard(error: SetupError) {
    Card(
        colors =
            CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.errorContainer,
                contentColor = MaterialTheme.colorScheme.onErrorContainer,
            ),
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(top = 8.dp)
                .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Text(
            text = stringResource(error.message()),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(16.dp),
        )
    }
}

@StringRes
internal fun ConnectionSecurity.label(): Int =
    when (this) {
        ConnectionSecurity.TLS -> R.string.setup_security_tls
        ConnectionSecurity.START_TLS -> R.string.setup_security_starttls
    }

@StringRes
internal fun FieldProblem.message(): Int =
    when (this) {
        FieldProblem.EMPTY -> R.string.setup_error_empty
        FieldProblem.CONTAINS_SPACE -> R.string.setup_error_contains_space
        FieldProblem.HAS_SCHEME -> R.string.setup_error_has_scheme
        FieldProblem.INVALID_CHARACTERS -> R.string.setup_error_invalid_characters
        FieldProblem.PORT_OUT_OF_RANGE -> R.string.setup_error_port_out_of_range
        FieldProblem.MALFORMED_EMAIL -> R.string.setup_error_malformed_email
    }

@StringRes
internal fun SetupError.message(): Int {
    // Separate strings per server, so translators can word each one naturally.
    fun pick(
        @StringRes imap: Int,
        @StringRes smtp: Int,
    ) = if (server == MailServer.SMTP) smtp else imap
    return when (failure) {
        SetupFailure.CANNOT_CONNECT ->
            pick(R.string.setup_error_cannot_connect_imap, R.string.setup_error_cannot_connect_smtp)
        SetupFailure.TLS_FAILURE -> pick(R.string.setup_error_tls_imap, R.string.setup_error_tls_smtp)
        SetupFailure.STARTTLS_NOT_OFFERED ->
            pick(R.string.setup_error_starttls_imap, R.string.setup_error_starttls_smtp)
        SetupFailure.AUTH_FAILED -> pick(R.string.setup_error_auth_imap, R.string.setup_error_auth_smtp)
        SetupFailure.TIMEOUT -> pick(R.string.setup_error_timeout_imap, R.string.setup_error_timeout_smtp)
        SetupFailure.SAVE_FAILED -> R.string.setup_error_save_failed
        SetupFailure.ALREADY_ADDED -> R.string.setup_error_already_added
        SetupFailure.GOOGLE_SIGN_IN_FAILED -> R.string.setup_error_google
        SetupFailure.UNEXPECTED -> R.string.setup_error_unexpected
    }
}
