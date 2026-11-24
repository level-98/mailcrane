package app.mailcrane.ui.account

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.AddAccountException
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.Field
import app.mailcrane.core.FieldProblem
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import app.mailcrane.core.ServerConfig
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import javax.inject.Inject

/** Why the last check failed, for the message under the form. */
enum class SetupFailure {
    CANNOT_CONNECT,
    TLS_FAILURE,
    STARTTLS_NOT_OFFERED,
    AUTH_FAILED,
    TIMEOUT,
    SAVE_FAILED,

    /** Anything the core did not classify, including an app set-up bug. */
    UNEXPECTED,
}

/** A failed check: what went wrong, and on which server (if one). */
data class SetupError(
    val failure: SetupFailure,
    val server: MailServer? = null,
)

/** Everything the manual setup screen shows. Holds what the user typed, so nothing is lost on errors. */
data class ManualSetupUiState(
    val email: String = "",
    val displayName: String = "",
    val imapHost: String = "",
    val imapPort: String = "",
    val imapSecurity: ConnectionSecurity = ConnectionSecurity.TLS,
    val smtpHost: String = "",
    val smtpPort: String = "",
    val smtpSecurity: ConnectionSecurity = ConnectionSecurity.TLS,
    val username: String = "",
    val password: String = "",
    val passwordVisible: Boolean = false,
    val checking: Boolean = false,
    val fieldErrors: Map<Field, FieldProblem> = emptyMap(),
    val error: SetupError? = null,
    val addedAccountId: Long? = null,
)

@HiltViewModel
class ManualSetupViewModel
    @Inject
    constructor(
        private val core: MailcraneCore,
    ) : ViewModel() {
        private val _state =
            MutableStateFlow(
                ManualSetupUiState(
                    imapPort = port(MailServer.IMAP, ConnectionSecurity.TLS),
                    smtpPort = port(MailServer.SMTP, ConnectionSecurity.TLS),
                ),
            )
        val state: StateFlow<ManualSetupUiState> = _state.asStateFlow()

        fun onEmailChange(value: String) = edit(Field.EMAIL) { copy(email = value) }

        fun onDisplayNameChange(value: String) = edit(Field.DISPLAY_NAME) { copy(displayName = value) }

        fun onImapHostChange(value: String) = edit(Field.IMAP_HOST) { copy(imapHost = value) }

        fun onImapPortChange(value: String) = edit(Field.IMAP_PORT) { copy(imapPort = digits(value)) }

        fun onSmtpHostChange(value: String) = edit(Field.SMTP_HOST) { copy(smtpHost = value) }

        fun onSmtpPortChange(value: String) = edit(Field.SMTP_PORT) { copy(smtpPort = digits(value)) }

        fun onUsernameChange(value: String) = edit(Field.USERNAME) { copy(username = value) }

        fun onPasswordChange(value: String) = edit(Field.PASSWORD) { copy(password = value) }

        fun onTogglePasswordVisible() = _state.update { it.copy(passwordVisible = !it.passwordVisible) }

        /** Changing security fills in that choice's usual port. */
        fun onImapSecurityChange(value: ConnectionSecurity) =
            edit(Field.IMAP_PORT) { copy(imapSecurity = value, imapPort = port(MailServer.IMAP, value)) }

        fun onSmtpSecurityChange(value: ConnectionSecurity) =
            edit(Field.SMTP_PORT) { copy(smtpSecurity = value, smtpPort = port(MailServer.SMTP, value)) }

        /** Checks the settings by logging in, and saves the account if both servers accept it. */
        fun submit() {
            val current = _state.value
            if (current.checking) return
            _state.update { it.copy(checking = true, error = null, fieldErrors = emptyMap()) }
            viewModelScope.launch {
                val result =
                    try {
                        Result.success(core.addAccountManual(current.toConfig(), current.password))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Result.failure(e)
                    }
                _state.update { state ->
                    result.fold(
                        onSuccess = { id -> state.copy(checking = false, addedAccountId = id) },
                        onFailure = { e -> state.withFailure(e) },
                    )
                }
            }
        }

        private fun ManualSetupUiState.withFailure(e: Throwable): ManualSetupUiState =
            when (e) {
                is AddAccountException.Invalid ->
                    copy(checking = false, fieldErrors = e.errors.associate { it.field to it.problem })
                else -> copy(checking = false, error = e.toSetupError())
            }

        private fun edit(
            field: Field,
            change: ManualSetupUiState.() -> ManualSetupUiState,
        ) = _state.update { it.change().copy(fieldErrors = it.fieldErrors - field) }

        private fun port(
            protocol: MailServer,
            security: ConnectionSecurity,
        ): String =
            runCatching { core.defaultPort(protocol, security).toString() }
                .getOrDefault("")

        private fun digits(value: String) = value.filter { it.isDigit() }.take(5)
    }

/** Ports that are empty or not a number reach the core as 0, which it reports as out of range. */
private fun String.toPort(): UInt = toUIntOrNull() ?: 0u

internal fun ManualSetupUiState.toConfig() =
    ManualAccountConfig(
        email = email.trim(),
        displayName = displayName.trim(),
        imap = ServerConfig(host = imapHost.trim(), port = imapPort.toPort(), security = imapSecurity),
        smtp = ServerConfig(host = smtpHost.trim(), port = smtpPort.toPort(), security = smtpSecurity),
        username = username.trim(),
    )

internal fun Throwable.toSetupError(): SetupError =
    when (this) {
        is AddAccountException.CannotConnect -> SetupError(SetupFailure.CANNOT_CONNECT, server)
        is AddAccountException.TlsFailure -> SetupError(SetupFailure.TLS_FAILURE, server)
        is AddAccountException.StartTlsNotOffered -> SetupError(SetupFailure.STARTTLS_NOT_OFFERED, server)
        is AddAccountException.AuthFailed -> SetupError(SetupFailure.AUTH_FAILED, server)
        is AddAccountException.Timeout -> SetupError(SetupFailure.TIMEOUT, server)
        is AddAccountException.SaveFailed -> SetupError(SetupFailure.SAVE_FAILED)
        else -> SetupError(SetupFailure.UNEXPECTED)
    }
