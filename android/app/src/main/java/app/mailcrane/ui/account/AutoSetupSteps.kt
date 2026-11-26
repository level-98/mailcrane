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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import app.mailcrane.core.Field
import app.mailcrane.core.ServerConfig
import app.mailcrane.core.SettingsSource

/** Step one of setup (M1-15): just the address and password. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun EmailStepContent(
    state: ManualSetupUiState,
    actions: ManualSetupActions,
) {
    val enabled = !state.discovering
    StepScaffold {
        Text(
            text = stringResource(R.string.setup_auto_intro),
            style = MaterialTheme.typography.bodyMedium,
        )
        FormField(
            value = state.email,
            onValueChange = actions.onEmailChange,
            label = R.string.setup_email,
            error = state.fieldErrors[Field.EMAIL],
            enabled = enabled,
            keyboardType = KeyboardType.Email,
        )
        if (state.gmail) {
            // Gmail needs Sign in with Google (M3-02): no password is typed here.
            Text(
                text = stringResource(R.string.setup_google_gmail_note),
                style = MaterialTheme.typography.bodyMedium,
            )
            ProgressButton(
                busy = state.checking,
                busyLabel = R.string.setup_google_signing_in,
                label = R.string.setup_google_sign_in,
                onClick = actions.onSignInWithGoogle,
            )
        } else {
            PasswordField(state, actions, enabled)
            Spacer(Modifier.height(8.dp))
            ProgressButton(
                busy = state.discovering,
                busyLabel = R.string.setup_auto_looking,
                label = R.string.setup_auto_continue,
                onClick = actions.onDiscover,
            )
            // Google Workspace accounts on their own domain.
            OutlinedButton(
                onClick = actions.onSignInWithGoogle,
                enabled = enabled && !state.checking,
                modifier = Modifier.fillMaxWidth(),
            ) { Text(stringResource(R.string.setup_google_sign_in)) }
        }
        state.error?.let { ErrorCard(it) }
        TextButton(
            onClick = actions.onSetUpManually,
            enabled = enabled,
            modifier = Modifier.fillMaxWidth(),
        ) { Text(stringResource(R.string.setup_auto_manual)) }
    }
}

/** Step two: what was found, for the user to confirm before logging in. */
@Composable
internal fun ConfirmStepContent(
    state: ManualSetupUiState,
    actions: ManualSetupActions,
) {
    val found = state.found
    StepScaffold {
        Text(
            text = stringResource(R.string.setup_auto_found_title),
            style = MaterialTheme.typography.titleMedium,
        )
        if (found != null) {
            Text(
                text = stringResource(found.source.label(), found.providerName ?: found.sourceDomain),
                style = MaterialTheme.typography.bodyMedium,
            )
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    ServerSummary(R.string.setup_section_imap, found.imap)
                    ServerSummary(R.string.setup_section_smtp, found.smtp)
                    Column {
                        Text(
                            text = stringResource(R.string.setup_username),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.primary,
                        )
                        Text(found.username, style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        }
        state.error?.let { ErrorCard(it) }
        Spacer(Modifier.height(8.dp))
        ProgressButton(
            busy = state.checking,
            busyLabel = R.string.setup_checking,
            label = R.string.setup_auto_use,
            onClick = actions.onSubmit,
        )
        OutlinedButton(
            onClick = actions.onEditSettings,
            enabled = !state.checking,
            modifier = Modifier.fillMaxWidth(),
        ) { Text(stringResource(R.string.setup_auto_edit)) }
    }
}

@Composable
private fun ServerSummary(
    @StringRes title: Int,
    server: ServerConfig,
) {
    Column {
        Text(
            text = stringResource(title),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
        )
        Text("${server.host}:${server.port}", style = MaterialTheme.typography.bodyLarge)
        Text(stringResource(server.security.label()), style = MaterialTheme.typography.bodyMedium)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun StepScaffold(content: @Composable () -> Unit) {
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
            content()
            Spacer(Modifier.height(16.dp))
        }
    }
}

@Composable
private fun PasswordField(
    state: ManualSetupUiState,
    actions: ManualSetupActions,
    enabled: Boolean,
) {
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
                val label = if (state.passwordVisible) R.string.setup_password_hide else R.string.setup_password_show
                Text(stringResource(label))
            }
        },
    )
}

@Composable
private fun ProgressButton(
    busy: Boolean,
    @StringRes busyLabel: Int,
    @StringRes label: Int,
    onClick: () -> Unit,
) {
    Button(onClick = onClick, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        if (busy) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.onPrimary,
                )
                Spacer(Modifier.size(12.dp))
                Text(stringResource(busyLabel))
            }
        } else {
            Text(stringResource(label))
        }
    }
}

@StringRes
private fun SettingsSource.label(): Int =
    when (this) {
        SettingsSource.AUTOCONFIG -> R.string.setup_auto_source_autoconfig
        SettingsSource.SRV -> R.string.setup_auto_source_srv
        SettingsSource.MX_AUTOCONFIG -> R.string.setup_auto_source_mx
        SettingsSource.ISPDB -> R.string.setup_auto_source_ispdb
    }
