package app.mailcrane.ui.account

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.AddAccountException
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.DiscoverException
import app.mailcrane.core.DiscoveredSettings
import app.mailcrane.core.Field
import app.mailcrane.core.FieldProblem
import app.mailcrane.core.JmapAccountSetup
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import app.mailcrane.core.OAuthFailure
import app.mailcrane.core.OAuthProviderConfig
import app.mailcrane.core.ServerConfig
import app.mailcrane.oauth.OAuthResult
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

    /** The same account is already in the app (#346). */
    ALREADY_ADDED,

    /** Sign in with Google didn't finish (M3-02): refused, no browser, or the sign-in expired. */
    GOOGLE_SIGN_IN_FAILED,

    /** Anything the core did not classify, including an app set-up bug. */
    UNEXPECTED,
}

/** A failed check: what went wrong, and on which server (if one). */
data class SetupError(
    val failure: SetupFailure,
    val server: MailServer? = null,
)

/**
 * Which step of setup is showing (M1-15): email and password first, then the settings that were
 * found for confirmation, or the full form when nothing was found or the user wants to edit.
 */
enum class SetupStep {
    EMAIL,
    CONFIRM,
    MANUAL,
}

/** Why the full form is showing after a lookup, for a note above it. */
enum class DiscoveryNote {
    NOT_FOUND,
    TIMED_OUT,
}

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
    val step: SetupStep = SetupStep.EMAIL,
    val discovering: Boolean = false,
    val found: DiscoveredSettings? = null,
    /** A gmail.com or googlemail.com address: Sign in with Google is the way in (M3-02). */
    val gmail: Boolean = false,
    /** Sign in with Google is open in the browser and nothing has come back yet. */
    val waitingForGoogle: Boolean = false,
    val discoveryNote: DiscoveryNote? = null,
    /** JMAP instead of IMAP+SMTP (M3-04): one server, a domain or session URL. */
    val jmap: Boolean = false,
    val jmapServer: String = "",
)

@HiltViewModel
class ManualSetupViewModel
    @Inject
    constructor(
        private val core: MailcraneCore,
    ) : ViewModel() {
        private val _state = MutableStateFlow(emptyForm())
        val state: StateFlow<ManualSetupUiState> = _state.asStateFlow()

        private fun emptyForm() =
            ManualSetupUiState(
                imapPort = port(MailServer.IMAP, ConnectionSecurity.TLS),
                smtpPort = port(MailServer.SMTP, ConnectionSecurity.TLS),
            )

        /**
         * The screen has handed the new account on: back to an empty form. Without a navigation graph
         * this view model belongs to the activity and is reused, so a kept `addedAccountId` closed the
         * next "Add account" at once, until the app restarted (#343).
         */
        fun accountHandled() {
            _state.value = emptyForm()
        }

        fun onEmailChange(value: String) =
            edit(Field.EMAIL) { copy(email = value, gmail = core.isGmailAddress(value.trim())) }

        fun onDisplayNameChange(value: String) = edit(Field.DISPLAY_NAME) { copy(displayName = value) }

        fun onImapHostChange(value: String) = edit(Field.IMAP_HOST) { copy(imapHost = value) }

        fun onImapPortChange(value: String) = edit(Field.IMAP_PORT) { copy(imapPort = digits(value)) }

        fun onSmtpHostChange(value: String) = edit(Field.SMTP_HOST) { copy(smtpHost = value) }

        fun onSmtpPortChange(value: String) = edit(Field.SMTP_PORT) { copy(smtpPort = digits(value)) }

        fun onUsernameChange(value: String) = edit(Field.USERNAME) { copy(username = value) }

        fun onPasswordChange(value: String) = edit(Field.PASSWORD) { copy(password = value) }

        fun onJmapChange(value: Boolean) =
            _state.update { it.copy(jmap = value, fieldErrors = emptyMap(), error = null) }

        // The core reports a bad JMAP server under the incoming-server field.
        fun onJmapServerChange(value: String) = edit(Field.IMAP_HOST) { copy(jmapServer = value) }

        fun onTogglePasswordVisible() = _state.update { it.copy(passwordVisible = !it.passwordVisible) }

        /** Changing security fills in that choice's usual port. */
        fun onImapSecurityChange(value: ConnectionSecurity) =
            edit(Field.IMAP_PORT) { copy(imapSecurity = value, imapPort = port(MailServer.IMAP, value)) }

        fun onSmtpSecurityChange(value: ConnectionSecurity) =
            edit(Field.SMTP_PORT) { copy(smtpSecurity = value, smtpPort = port(MailServer.SMTP, value)) }

        /**
         * Looks up the settings for the typed address. Found settings fill the form and show for
         * confirmation; otherwise the full form opens with a note.
         */
        fun discover() {
            val current = _state.value
            if (current.discovering || current.checking) return
            val email = current.email.trim()
            val problems =
                buildMap {
                    if (!email.contains('@')) put(Field.EMAIL, FieldProblem.MALFORMED_EMAIL)
                    if (current.password.isEmpty()) put(Field.PASSWORD, FieldProblem.EMPTY)
                }
            if (problems.isNotEmpty()) {
                _state.update { it.copy(fieldErrors = problems) }
                return
            }
            _state.update { it.copy(discovering = true, error = null, fieldErrors = emptyMap(), discoveryNote = null) }
            viewModelScope.launch {
                val result =
                    try {
                        Result.success(core.discoverSettings(email))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Result.failure(e)
                    }
                val first = result.getOrNull()?.firstOrNull()
                _state.update { state ->
                    when {
                        first != null -> state.withFound(first)
                        result.exceptionOrNull() is DiscoverException.InvalidEmail ->
                            state.copy(
                                discovering = false,
                                fieldErrors =
                                    mapOf(
                                        Field.EMAIL to FieldProblem.MALFORMED_EMAIL,
                                    ),
                            )
                        else ->
                            state.copy(
                                discovering = false,
                                step = SetupStep.MANUAL,
                                username = state.username.ifBlank { email },
                                discoveryNote =
                                    if (result.exceptionOrNull() is DiscoverException.Timeout) {
                                        DiscoveryNote.TIMED_OUT
                                    } else {
                                        DiscoveryNote.NOT_FOUND
                                    },
                            )
                    }
                }
            }
        }

        /** Shows the full form, with anything found already filled in. */
        fun editSettings() = _state.update { it.copy(step = SetupStep.MANUAL, discoveryNote = null) }

        /** Skips the lookup: the full form with the username filled from the address. */
        fun setUpManually() =
            _state.update {
                it.copy(
                    step = SetupStep.MANUAL,
                    username = it.username.ifBlank { it.email.trim() },
                    fieldErrors = emptyMap(),
                )
            }

        /**
         * Starts Sign in with Google (M3-02) for the typed address: returns the provider and the
         * address to open in the browser, or null (with a field error) if there's no address yet.
         */
        fun startGoogleSignIn(): Pair<OAuthProviderConfig, String>? {
            val current = _state.value
            if (current.checking) return null
            val email = current.email.trim()
            if (!email.contains('@')) {
                _state.update { it.copy(fieldErrors = mapOf(Field.EMAIL to FieldProblem.MALFORMED_EMAIL)) }
                return null
            }
            val provider =
                try {
                    core.googleOAuthProvider()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    _state.update { it.copy(error = SetupError(SetupFailure.GOOGLE_SIGN_IN_FAILED)) }
                    return null
                }
            _state.update {
                it.copy(checking = true, waitingForGoogle = true, error = null, fieldErrors = emptyMap())
            }
            return provider to email
        }

        /**
         * The app is in front again but the browser sent nothing back: the user closed the tab.
         * Back to the form, quietly. Called a moment after the app resumes, so a redirect that's
         * on its way still wins.
         */
        fun googleNotReturned() {
            _state.update {
                if (it.waitingForGoogle) it.copy(checking = false, waitingForGoogle = false) else it
            }
        }

        /** How Sign in with Google ended in the browser: on success, the account is checked and saved. */
        fun onGoogleResult(result: OAuthResult) {
            when (result) {
                // The browser came back; the code is being exchanged.
                OAuthResult.Returned -> _state.update { it.copy(waitingForGoogle = false) }
                is OAuthResult.Failed -> {
                    // Cancelled in the browser: back to the form, quietly.
                    val cancelled = result.reason is OAuthFailure.Denied
                    _state.update {
                        it.copy(
                            checking = false,
                            waitingForGoogle = false,
                            error = if (cancelled) null else SetupError(SetupFailure.GOOGLE_SIGN_IN_FAILED),
                        )
                    }
                }
                is OAuthResult.SignedIn -> {
                    val current = _state.value
                    _state.update { it.copy(checking = true, waitingForGoogle = false, error = null) }
                    viewModelScope.launch {
                        val added =
                            try {
                                Result.success(
                                    core.addAccountGoogle(
                                        result.signInId,
                                        current.email.trim(),
                                        current.displayName.trim(),
                                    ),
                                )
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                Result.failure(e)
                            }
                        _state.update { state ->
                            added.fold(
                                onSuccess = { id -> state.copy(checking = false, addedAccountId = id) },
                                onFailure = { e -> state.withFailure(e) },
                            )
                        }
                    }
                }
            }
        }

        /** Checks the settings by logging in, and saves the account if both servers accept it. */
        fun submit() {
            val current = _state.value
            if (current.checking) return
            _state.update { it.copy(checking = true, error = null, fieldErrors = emptyMap()) }
            viewModelScope.launch {
                val result =
                    try {
                        if (current.jmap) {
                            Result.success(core.addAccountJmap(current.toJmapSetup(), current.password))
                        } else {
                            Result.success(core.addAccountManual(current.toConfig(), current.password))
                        }
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

        private fun ManualSetupUiState.withFound(found: DiscoveredSettings) =
            copy(
                discovering = false,
                step = SetupStep.CONFIRM,
                found = found,
                imapHost = found.imap.host,
                imapPort = found.imap.port.toString(),
                imapSecurity = found.imap.security,
                smtpHost = found.smtp.host,
                smtpPort = found.smtp.port.toString(),
                smtpSecurity = found.smtp.security,
                username = found.username,
            )

        private fun ManualSetupUiState.withFailure(e: Throwable): ManualSetupUiState =
            when (e) {
                is AddAccountException.Invalid ->
                    copy(
                        checking = false,
                        step = SetupStep.MANUAL,
                        fieldErrors = e.errors.associate { it.field to it.problem },
                    )
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

internal fun ManualSetupUiState.toJmapSetup() =
    JmapAccountSetup(
        email = email.trim(),
        displayName = displayName.trim(),
        server = jmapServer.trim(),
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
        is AddAccountException.AlreadyAdded -> SetupError(SetupFailure.ALREADY_ADDED)
        else -> SetupError(SetupFailure.UNEXPECTED)
    }
