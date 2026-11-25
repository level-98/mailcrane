package app.mailcrane.ui.compose

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.DraftKind
import app.mailcrane.core.RecipientEntry
import app.mailcrane.core.RecipientField
import app.mailcrane.core.RecipientProblem
import app.mailcrane.core.ReplyKind
import app.mailcrane.core.SendFailure
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import javax.inject.Inject

/** One recipient chip. [text] is what the user typed and goes back to the core unchanged. */
data class RecipientChip(
    val text: String,
    val name: String,
    val email: String,
    val problem: RecipientProblem?,
) {
    val isValid: Boolean get() = problem == null

    /** What the chip shows: the name, else the address, else the text as typed. */
    val label: String get() = name.ifBlank { email }.ifBlank { text }
}

/** A To, Cc or Bcc field: finished chips and the text still being typed. */
data class RecipientFieldState(
    val chips: List<RecipientChip> = emptyList(),
    val input: String = "",
    /** Whether [input], if finished now, would be at least one valid address. */
    val inputValid: Boolean = false,
) {
    val hasValid: Boolean get() = inputValid || chips.any { it.isValid }
    val isEmpty: Boolean get() = chips.isEmpty() && input.isBlank()
}

/** Why sending failed. The message stays on screen in every case. */
enum class ComposeError {
    INVALID_RECIPIENTS,
    NO_RECIPIENTS,
    TOO_MANY_RECIPIENTS,
    TOO_LARGE,
    ACCOUNT_NOT_FOUND,
    INVALID_SENDER,
    MISSING_CREDENTIALS,
    STORAGE,
    CANNOT_CONNECT,
    TLS,
    STARTTLS_NOT_OFFERED,
    AUTH,
    TIMEOUT,
    ALL_REJECTED,
    REFUSED_TEMPORARY,
    REFUSED_PERMANENT,
    UNEXPECTED,

    /** The message being replied to couldn't be loaded; the screen is empty but usable. */
    REPLY_UNAVAILABLE,
}

/** How the screen ended. */
sealed interface ComposeResult {
    /** Sent; [rejected] lists addresses the server refused while accepting the rest. */
    data class Sent(
        val rejected: List<String>,
    ) : ComposeResult

    /** Closed with nothing typed, or after the user confirmed discarding. */
    data object Closed : ComposeResult
}

/** Everything the compose screen shows. Holds all typed content, so nothing is lost on errors. */
data class ComposeUiState(
    val accounts: List<AccountSummary> = emptyList(),
    val fromAccountId: Long? = null,
    val to: RecipientFieldState = RecipientFieldState(),
    val cc: RecipientFieldState = RecipientFieldState(),
    val bcc: RecipientFieldState = RecipientFieldState(),
    val showCcBcc: Boolean = false,
    val subject: String = "",
    val body: String = "",
    val sending: Boolean = false,
    val error: ComposeError? = null,
    /** Addresses named in [error], for "all recipients were refused". */
    val errorAddresses: List<String> = emptyList(),
    val confirmingDiscard: Boolean = false,
    val result: ComposeResult? = null,
    /** For a reply: the Message-ID replied to, sent as `In-Reply-To`. */
    val inReplyTo: String? = null,
    /** For a reply: the thread's Message-IDs, sent as `References`. */
    val references: List<String> = emptyList(),
    /** The reply this screen was filled for, so rotation doesn't fill it again. */
    val prefilledFor: ReplyRequest? = null,
    /** The signature put into [body], so a change of From can swap it if it wasn't edited. */
    val signature: String? = null,
    /** Whether the signature was added already (once per message, not again on rotation). */
    val signatureAdded: Boolean = false,
) {
    fun field(which: RecipientField) =
        when (which) {
            RecipientField.TO -> to
            RecipientField.CC -> cc
            RecipientField.BCC -> bcc
        }

    val hasValidRecipient: Boolean get() = to.hasValid || cc.hasValid || bcc.hasValid

    val canSend: Boolean get() = !sending && fromAccountId != null && hasValidRecipient

    val hasContent: Boolean
        get() = !to.isEmpty || !cc.isEmpty || !bcc.isEmpty || subject.isNotBlank() || body.isNotBlank()

    /** The From picker is hidden when there is only one account. */
    val showFromPicker: Boolean get() = accounts.size > 1
}

@HiltViewModel
class ComposeViewModel
    @Inject
    constructor(
        private val core: ComposeCore,
    ) : ViewModel() {
        private val _state = MutableStateFlow(ComposeUiState())
        val state: StateFlow<ComposeUiState> = _state.asStateFlow()

        /**
         * Opens the screen for [accountId]. Keeps what is already typed (e.g. after rotation) and only
         * picks the sender if none is picked yet.
         */
        fun start(
            accountId: Long,
            reply: ReplyRequest? = null,
        ) {
            _state.update { if (it.fromAccountId == null) it.copy(fromAccountId = accountId) else it }
            if (reply != null && _state.value.prefilledFor != reply) prefill(reply)
            if (reply == null && !_state.value.signatureAdded) addSignature(accountId)
            viewModelScope.launch {
                val accounts =
                    try {
                        core.listAccounts()
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        emptyList()
                    }
                _state.update { s ->
                    // Keep the picked account if it still exists; otherwise fall back to the first one.
                    val from = s.fromAccountId?.takeIf { id -> accounts.isEmpty() || accounts.any { it.id == id } }
                    s.copy(accounts = accounts, fromAccountId = from ?: accounts.firstOrNull()?.id)
                }
            }
        }

        /** Adds the From account's signature to a new message, once. */
        private fun addSignature(accountId: Long) {
            _state.update { it.copy(signatureAdded = true) }
            viewModelScope.launch {
                val signature = core.signatureFor(accountId) ?: return@launch
                _state.update {
                    it.copy(body = core.applySignature(it.body, signature, DraftKind.NEW), signature = signature)
                }
            }
        }

        /** Fills the screen with the core's reply draft: recipients as chips, subject, quoted body. */
        private fun prefill(reply: ReplyRequest) {
            _state.update { it.copy(prefilledFor = reply, signatureAdded = true) }
            viewModelScope.launch {
                val draft =
                    try {
                        core.prepareReply(reply.messageId, reply.kind, reply.quoteHeader)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        _state.update { it.copy(error = ComposeError.REPLY_UNAVAILABLE) }
                        return@launch
                    }
                val cc = chipsOf(draft.cc)
                val signature = core.signatureFor(draft.accountId)
                val kind = if (reply.kind == ReplyKind.FORWARD) DraftKind.FORWARD else DraftKind.REPLY
                val body = core.applySignature(draft.bodyText, signature, kind)
                _state.update {
                    it.copy(
                        fromAccountId = draft.accountId,
                        to = chipsOf(draft.to),
                        cc = cc,
                        showCcBcc = it.showCcBcc || !cc.isEmpty,
                        subject = draft.subject,
                        body = body,
                        signature = signature,
                        inReplyTo = draft.inReplyTo,
                        references = draft.references,
                    )
                }
            }
        }

        private fun chipsOf(entries: List<String>): RecipientFieldState =
            RecipientFieldState(
                chips = entries.flatMap { entry -> parse(entry, whole = true)?.entries?.map { it.toChip() }.orEmpty() },
            )

        /** Picks the sender, swapping in its signature if the old one wasn't edited. */
        fun onFromChange(accountId: Long) {
            val before = _state.value.fromAccountId
            _state.update { it.copy(fromAccountId = accountId, error = null) }
            if (before == accountId) return
            viewModelScope.launch {
                val signature = core.signatureFor(accountId)
                _state.update {
                    val swapped = core.swapSignature(it.body, it.signature, signature)
                    // An edited signature stays, and keeps being the one to match.
                    val kept = swapped == it.body && it.signature != null && it.signature != signature
                    it.copy(body = swapped, signature = if (kept) it.signature else signature)
                }
            }
        }

        /** Text typed into a recipient field. Finished entries (followed by a comma or semicolon) become chips. */
        fun onRecipientInput(
            which: RecipientField,
            text: String,
        ) = editField(which) { field ->
            val parsed = parse(text, whole = false) ?: return@editField field.copy(input = text, inputValid = false)
            val rest = parsed.rest
            field.copy(
                chips = field.chips + parsed.entries.map { it.toChip() },
                input = rest,
                inputValid = isValidInput(rest),
            )
        }

        /** Turns what is left in the field into chips, e.g. when it loses focus or Enter is pressed. */
        fun commitRecipient(which: RecipientField) = editField(which) { it.committed() }

        fun removeRecipient(
            which: RecipientField,
            index: Int,
        ) = editField(which) { field ->
            if (index !in
                field.chips.indices
            ) {
                field
            } else {
                field.copy(chips = field.chips.filterIndexed { i, _ -> i != index })
            }
        }

        /** Backspace in an empty field puts the last chip's text back for editing. */
        fun editLastRecipient(which: RecipientField) =
            editField(which) { field ->
                val last = field.chips.lastOrNull()
                if (field.input.isNotEmpty() || last == null) {
                    field
                } else {
                    field.copy(chips = field.chips.dropLast(1), input = last.text, inputValid = last.isValid)
                }
            }

        fun showCcBcc() = _state.update { it.copy(showCcBcc = true) }

        fun onSubjectChange(value: String) = _state.update { it.copy(subject = value) }

        fun onBodyChange(value: String) = _state.update { it.copy(body = value) }

        fun dismissError() = _state.update { it.copy(error = null, errorAddresses = emptyList()) }

        fun send() {
            val current = _state.value
            if (!current.canSend) return
            val committed =
                current.copy(
                    to = current.to.committed(),
                    cc = current.cc.committed(),
                    bcc = current.bcc.committed(),
                    error = null,
                    errorAddresses = emptyList(),
                )
            val accountId = committed.fromAccountId ?: return
            val all = committed.to.chips + committed.cc.chips + committed.bcc.chips
            if (all.any { !it.isValid }) {
                _state.value =
                    committed.copy(
                        error = ComposeError.INVALID_RECIPIENTS,
                        showCcBcc =
                            committed.showCcBcc || hasCcBcc(committed),
                    )
                return
            }
            _state.value = committed.copy(sending = true)
            val draft =
                ComposeDraft(
                    accountId = accountId,
                    to = committed.to.chips.map { it.text },
                    cc = committed.cc.chips.map { it.text },
                    bcc = committed.bcc.chips.map { it.text },
                    subject = committed.subject,
                    bodyText = committed.body,
                    inReplyTo = committed.inReplyTo,
                    references = committed.references,
                )
            viewModelScope.launch {
                val outcome =
                    try {
                        Result.success(core.send(draft))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Result.failure(e)
                    }
                _state.update { s ->
                    outcome.fold(
                        onSuccess = { s.copy(sending = false, result = ComposeResult.Sent(it.rejected)) },
                        onFailure = { e -> s.withFailure(e) },
                    )
                }
            }
        }

        /** Close or Back. Asks first if anything has been typed; does nothing while sending. */
        fun requestClose() =
            _state.update { s ->
                when {
                    s.sending -> s
                    s.hasContent -> s.copy(confirmingDiscard = true)
                    else -> s.copy(result = ComposeResult.Closed)
                }
            }

        fun confirmDiscard() = _state.update { it.copy(confirmingDiscard = false, result = ComposeResult.Closed) }

        fun keepEditing() = _state.update { it.copy(confirmingDiscard = false) }

        /** Clears the message once the screen has handled [ComposeUiState.result], ready for the next one. */
        fun reset() = _state.update { ComposeUiState(accounts = it.accounts) }

        private fun hasCcBcc(s: ComposeUiState) = !s.cc.isEmpty || !s.bcc.isEmpty

        private fun ComposeUiState.withFailure(e: Throwable): ComposeUiState {
            val base = copy(sending = false)
            if (e !is SendFailure) return base.copy(error = ComposeError.UNEXPECTED)
            return when (e) {
                is SendFailure.InvalidRecipients -> base.markInvalid(e)
                is SendFailure.RecipientsRejected ->
                    base.copy(error = ComposeError.ALL_REJECTED, errorAddresses = e.rejected)
                is SendFailure.Refused ->
                    base.copy(
                        error = if (e.temporary) ComposeError.REFUSED_TEMPORARY else ComposeError.REFUSED_PERMANENT,
                    )
                else -> base.copy(error = e.toComposeError())
            }
        }

        /** Marks the chips the core found invalid (normally already marked when they were typed). */
        private fun ComposeUiState.markInvalid(e: SendFailure.InvalidRecipients): ComposeUiState {
            fun mark(
                which: RecipientField,
                field: RecipientFieldState,
            ): RecipientFieldState {
                val bad = e.errors.filter { it.field == which }.associate { it.entry.toInt() to it.problem }
                return field.copy(
                    chips =
                        field.chips.mapIndexed { i, chip ->
                            bad[i]?.let { chip.copy(problem = it) }
                                ?: chip
                        },
                )
            }
            val marked =
                copy(
                    to = mark(RecipientField.TO, to),
                    cc = mark(RecipientField.CC, cc),
                    bcc = mark(RecipientField.BCC, bcc),
                    error = ComposeError.INVALID_RECIPIENTS,
                )
            return marked.copy(showCcBcc = marked.showCcBcc || hasCcBcc(marked))
        }

        private fun editField(
            which: RecipientField,
            change: (RecipientFieldState) -> RecipientFieldState,
        ) = _state.update { s ->
            when (which) {
                RecipientField.TO -> s.copy(to = change(s.to))
                RecipientField.CC -> s.copy(cc = change(s.cc))
                RecipientField.BCC -> s.copy(bcc = change(s.bcc))
            }
        }

        private fun RecipientFieldState.committed(): RecipientFieldState {
            if (input.isBlank()) return copy(input = "", inputValid = false)
            val parsed = parse(input, whole = true) ?: return this
            return copy(chips = chips + parsed.entries.map { it.toChip() }, input = "", inputValid = false)
        }

        private fun isValidInput(text: String): Boolean {
            if (text.isBlank()) return false
            val entries = parse(text, whole = true)?.entries ?: return false
            return entries.isNotEmpty() && entries.all { it.problem == null }
        }

        /** The core never throws here; `null` only guards against a broken native library. */
        private fun parse(
            text: String,
            whole: Boolean,
        ) = runCatching { core.parseRecipients(text, whole) }.getOrNull()
    }

private fun RecipientEntry.toChip() = RecipientChip(text = text, name = name, email = email, problem = problem)

internal fun SendFailure.toComposeError(): ComposeError =
    when (this) {
        is SendFailure.InvalidRecipients -> ComposeError.INVALID_RECIPIENTS
        is SendFailure.NoRecipients -> ComposeError.NO_RECIPIENTS
        is SendFailure.TooManyRecipients -> ComposeError.TOO_MANY_RECIPIENTS
        is SendFailure.TooLarge -> ComposeError.TOO_LARGE
        is SendFailure.AccountNotFound -> ComposeError.ACCOUNT_NOT_FOUND
        is SendFailure.InvalidSender -> ComposeError.INVALID_SENDER
        is SendFailure.MissingCredentials -> ComposeError.MISSING_CREDENTIALS
        is SendFailure.Storage -> ComposeError.STORAGE
        is SendFailure.CannotConnect -> ComposeError.CANNOT_CONNECT
        is SendFailure.TlsFailure -> ComposeError.TLS
        is SendFailure.StartTlsNotOffered -> ComposeError.STARTTLS_NOT_OFFERED
        is SendFailure.AuthFailed -> ComposeError.AUTH
        is SendFailure.Timeout -> ComposeError.TIMEOUT
        is SendFailure.RecipientsRejected -> ComposeError.ALL_REJECTED
        is SendFailure.Refused -> if (temporary) ComposeError.REFUSED_TEMPORARY else ComposeError.REFUSED_PERMANENT
        is SendFailure.Internal, is SendFailure.NotReady -> ComposeError.UNEXPECTED
    }
