package app.mailcrane.ui.compose

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.contacts.DeviceContacts
import app.mailcrane.contacts.NoDeviceContacts
import app.mailcrane.contacts.RecipientSuggestion
import app.mailcrane.contacts.mergeSuggestions
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeAttachment
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.DraftKind
import app.mailcrane.core.RecipientEntry
import app.mailcrane.core.RecipientField
import app.mailcrane.core.RecipientProblem
import app.mailcrane.core.ReplyKind
import app.mailcrane.core.SendFailure
import app.mailcrane.settings.AppSettings
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
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

    /** The draft couldn't be opened. */
    DRAFT_UNAVAILABLE,
}

/** How the screen ended. */
sealed interface ComposeResult {
    /** Sent; [rejected] lists addresses the server refused while accepting the rest. */
    data class Sent(
        val rejected: List<String>,
    ) : ComposeResult

    /** Closed with nothing typed, or after the user confirmed discarding. */
    data object Closed : ComposeResult

    /** Closed with the message kept as a draft. */
    data object SavedDraft : ComposeResult

    /** Not sent yet: in the outbox, sent when the network or server is back. */
    data object Queued : ComposeResult

    /** Scheduled to go out at [sendAt] (Unix seconds). */
    data class Scheduled(
        val sendAt: Long,
    ) : ComposeResult

    /** Waiting [seconds] before it goes out, so it can still be taken back (undo send). */
    data class Undoable(
        val outboxId: Long,
        val seconds: Int,
    ) : ComposeResult
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
    /** The saved draft this message is, once saved or opened. */
    val draftId: Long? = null,
    /** The draft request this screen was filled for, so rotation doesn't fill it again. */
    val openedDraft: DraftRequest? = null,
    /** Suggested recipients for what is being typed in [suggestionsFor] (M2-08). */
    val suggestions: List<RecipientSuggestion> = emptyList(),
    val suggestionsFor: RecipientField? = null,
    /** Ask once whether to suggest from the device's contacts. */
    val askContacts: Boolean = false,
    /** The fields this screen was filled with, so rotation doesn't fill them again. */
    val prefilledWith: ComposePrefill? = null,
    /** Files to attach (M2-04). */
    val attachments: List<ComposeAttachment> = emptyList(),
    /** A forward: the original message, whose attachments go along. */
    val forwardAttachmentsOf: Long? = null,
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
        private val settings: AppSettings,
        private val deviceContacts: DeviceContacts = NoDeviceContacts,
    ) : ViewModel() {
        private var suggesting: Job? = null
        private val _state = MutableStateFlow(ComposeUiState())
        val state: StateFlow<ComposeUiState> = _state.asStateFlow()

        /**
         * Opens the screen for [accountId]. Keeps what is already typed (e.g. after rotation) and only
         * picks the sender if none is picked yet.
         */
        fun start(
            accountId: Long,
            reply: ReplyRequest? = null,
            draft: DraftRequest? = null,
            prefill: ComposePrefill? = null,
        ) {
            _state.update { if (it.fromAccountId == null) it.copy(fromAccountId = accountId) else it }
            if (prefill != null && _state.value.prefilledWith != prefill) {
                _state.update {
                    it.copy(
                        prefilledWith = prefill,
                        to = chipsOf(listOf(prefill.to)),
                        subject = prefill.subject,
                        body = prefill.body,
                        // No signature in a message to a machine.
                        signatureAdded = true,
                    )
                }
            }
            if (draft != null) {
                if (_state.value.openedDraft != draft) openDraft(draft)
            } else if (prefill == null) {
                if (reply != null && _state.value.prefilledFor != reply) prefill(reply)
                if (reply == null && !_state.value.signatureAdded) addSignature(accountId)
            }
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

        /** Fills the screen from a saved draft (opening a Drafts folder message first if needed). */
        private fun openDraft(request: DraftRequest) {
            _state.update { it.copy(openedDraft = request, signatureAdded = true) }
            viewModelScope.launch {
                val loaded =
                    try {
                        val id = request.draftId ?: request.messageId?.let { core.openDraft(it) } ?: return@launch
                        id to core.getDraft(id)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        _state.update { it.copy(error = ComposeError.DRAFT_UNAVAILABLE) }
                        return@launch
                    }
                val (id, draft) = loaded
                val cc = chipsOf(draft.cc)
                val bcc = chipsOf(draft.bcc)
                _state.update {
                    it.copy(
                        draftId = id,
                        fromAccountId = draft.accountId,
                        to = chipsOf(draft.to),
                        cc = cc,
                        bcc = bcc,
                        showCcBcc = it.showCcBcc || !cc.isEmpty || !bcc.isEmpty,
                        subject = draft.subject,
                        body = draft.bodyText,
                        inReplyTo = draft.inReplyTo,
                        references = draft.references,
                        forwardAttachmentsOf = draft.forwardAttachmentsOf,
                    )
                }
            }
        }

        private var saveJob: Job? = null

        /** Saves a draft [AUTOSAVE_MS] after the last edit. */
        private fun scheduleSave() {
            saveJob?.cancel()
            saveJob =
                viewModelScope.launch {
                    delay(AUTOSAVE_MS)
                    saveNow()
                }
        }

        /** Saves the message as a draft now, if there is anything to keep. Returns whether it is saved. */
        private suspend fun saveNow(): Boolean {
            val s = _state.value
            if (!s.hasContent || s.sending || s.result != null) return false
            val draft = s.toDraft() ?: return false
            return try {
                val id = core.saveDraft(s.draftId, draft)
                _state.update { it.copy(draftId = id) }
                true
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // It stays on screen; the next edit or close tries again.
                false
            }
        }

        private fun ComposeUiState.toDraft(): ComposeDraft? {
            val account = fromAccountId ?: return null

            // Half-typed recipients are kept too.
            fun entries(f: RecipientFieldState) = f.chips.map { it.text } + listOf(f.input).filter { it.isNotBlank() }
            return ComposeDraft(
                accountId = account,
                to = entries(to),
                cc = entries(cc),
                bcc = entries(bcc),
                subject = subject,
                bodyText = body,
                inReplyTo = inReplyTo,
                references = references,
                attachments = attachments,
                forwardAttachmentsOf = forwardAttachmentsOf,
            )
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
                        forwardAttachmentsOf = draft.forwardAttachmentsOf,
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
        ) {
            editField(which) { field ->
                val parsed = parse(text, whole = false) ?: return@editField field.copy(input = text, inputValid = false)
                val rest = parsed.rest
                field.copy(
                    chips = field.chips + parsed.entries.map { it.toChip() },
                    input = rest,
                    inputValid = isValidInput(rest),
                )
            }
            suggest(which, _state.value.field(which).input)
        }

        /** Looks up recipients for [prefix]: past correspondents, then device contacts if allowed. */
        private fun suggest(
            which: RecipientField,
            prefix: String,
        ) {
            suggesting?.cancel()
            if (prefix.isBlank()) {
                clearSuggestions()
                return
            }
            suggesting =
                viewModelScope.launch {
                    if (!deviceContacts.available && !settings.contactsPromptAnswered.first()) {
                        _state.update { it.copy(askContacts = true) }
                    }
                    val account = _state.value.fromAccountId
                    val past =
                        core.suggestContacts(account, prefix, MAX_SUGGESTIONS).map {
                            RecipientSuggestion(it.name, it.email)
                        }
                    val device = deviceContacts.search(prefix, MAX_SUGGESTIONS)
                    val merged = mergeSuggestions(past, device, MAX_SUGGESTIONS)
                    _state.update {
                        it.copy(
                            suggestions = merged,
                            suggestionsFor = which.takeIf { merged.isNotEmpty() },
                        )
                    }
                }
        }

        private fun clearSuggestions() {
            suggesting?.cancel()
            _state.update { it.copy(suggestions = emptyList(), suggestionsFor = null) }
        }

        /** The user tapped a suggestion: it replaces what was typed with a chip. */
        fun pickSuggestion(
            which: RecipientField,
            suggestion: RecipientSuggestion,
        ) {
            val name = suggestion.name.replace("\"", "").trim()
            val text = if (name.isEmpty()) suggestion.email else "\"$name\" <${suggestion.email}>"
            editField(which) { field ->
                val chips = parse(text, whole = true)?.entries?.map { it.toChip() }.orEmpty()
                field.copy(chips = field.chips + chips, input = "", inputValid = false)
            }
            clearSuggestions()
        }

        /** Adds picked files to the message. */
        fun addAttachments(files: List<ComposeAttachment>) {
            if (files.isEmpty()) return
            _state.update { it.copy(attachments = it.attachments + files) }
        }

        fun removeAttachment(index: Int) {
            _state.update { s -> s.copy(attachments = s.attachments.filterIndexed { i, _ -> i != index }) }
        }

        /** The contacts question was answered (either way); it isn't asked again. */
        fun contactsAnswered() {
            _state.update { it.copy(askContacts = false) }
            viewModelScope.launch { settings.setContactsPromptAnswered() }
        }

        /** Turns what is left in the field into chips, e.g. when it loses focus or Enter is pressed. */
        fun commitRecipient(which: RecipientField) {
            editField(which) { it.committed() }
            clearSuggestions()
        }

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

        fun onSubjectChange(value: String) {
            _state.update { it.copy(subject = value) }
            scheduleSave()
        }

        fun onBodyChange(value: String) {
            _state.update { it.copy(body = value) }
            scheduleSave()
        }

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
            saveJob?.cancel()
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
                    attachments = committed.attachments,
                    forwardAttachmentsOf = committed.forwardAttachmentsOf,
                )
            viewModelScope.launch {
                val delay = settings.undoSendSeconds.first()
                if (delay > 0) {
                    sendLater(draft, delay, committed.draftId)
                    return@launch
                }
                val outcome =
                    try {
                        Result.success(core.send(draft))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Result.failure(e)
                    }
                if (outcome.isSuccess) committed.draftId?.let { deleteQuietly(it) }
                _state.update { s ->
                    outcome.fold(
                        onSuccess = {
                            s.copy(
                                sending = false,
                                result = if (it.queued) ComposeResult.Queued else ComposeResult.Sent(it.rejected),
                            )
                        },
                        onFailure = { e -> s.withFailure(e) },
                    )
                }
            }
        }

        /** Scheduled send: queues the message for [sendAt] (Unix seconds) and closes. */
        fun schedule(sendAt: Long) {
            val current = _state.value
            if (!current.canSend) return
            val committed =
                current.copy(to = current.to.committed(), cc = current.cc.committed(), bcc = current.bcc.committed())
            val accountId = committed.fromAccountId ?: return
            if ((committed.to.chips + committed.cc.chips + committed.bcc.chips).any { !it.isValid }) {
                _state.value = committed.copy(error = ComposeError.INVALID_RECIPIENTS)
                return
            }
            saveJob?.cancel()
            _state.value = committed.copy(sending = true, error = null)
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
                    attachments = committed.attachments,
                    forwardAttachmentsOf = committed.forwardAttachmentsOf,
                )
            viewModelScope.launch {
                try {
                    core.scheduleMessage(draft, sendAt)
                    committed.draftId?.let { deleteQuietly(it) }
                    _state.update { it.copy(sending = false, result = ComposeResult.Scheduled(sendAt)) }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    _state.update { it.withFailure(e) }
                }
            }
        }

        /** Undo send: queues the message for [delay] seconds and closes; the screen offers Undo meanwhile. */
        private suspend fun sendLater(
            draft: ComposeDraft,
            delay: Int,
            draftId: Long?,
        ) {
            try {
                val id = core.enqueueWithDelay(draft, delay)
                draftId?.let { deleteQuietly(it) }
                _state.update { it.copy(sending = false, result = ComposeResult.Undoable(id, delay)) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.withFailure(e) }
            }
        }

        /**
         * Close or Back: keeps what was typed as a draft and closes. With nothing typed it just closes (and
         * deletes a draft that was emptied). Does nothing while sending.
         */
        fun requestClose() {
            val s = _state.value
            if (s.sending || s.result != null) return
            saveJob?.cancel()
            viewModelScope.launch {
                if (s.hasContent) {
                    // If it can't be saved, ask before anything typed is lost.
                    val saved = saveNow()
                    _state.update {
                        if (saved) it.copy(result = ComposeResult.SavedDraft) else it.copy(confirmingDiscard = true)
                    }
                } else {
                    s.draftId?.let { deleteQuietly(it) }
                    _state.update { it.copy(result = ComposeResult.Closed) }
                }
            }
        }

        /** The Discard button: asks first when there is anything to lose. */
        fun requestDiscard() =
            _state.update { s ->
                when {
                    s.sending -> s
                    s.hasContent || s.draftId != null -> s.copy(confirmingDiscard = true)
                    else -> s.copy(result = ComposeResult.Closed)
                }
            }

        /** Discards the message and its saved draft. */
        fun confirmDiscard() {
            saveJob?.cancel()
            val id = _state.value.draftId
            _state.update { it.copy(confirmingDiscard = false) }
            viewModelScope.launch {
                id?.let { deleteQuietly(it) }
                _state.update { it.copy(result = ComposeResult.Closed) }
            }
        }

        private suspend fun deleteQuietly(id: Long) {
            try {
                core.deleteDraft(id)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // An orphaned draft is better than a crash; it shows in the drafts list.
            }
        }

        fun keepEditing() = _state.update { it.copy(confirmingDiscard = false) }

        /** Clears the message once the screen has handled [ComposeUiState.result], ready for the next one. */
        fun reset() = _state.update { ComposeUiState(accounts = it.accounts) }

        private fun hasCcBcc(s: ComposeUiState) = !s.cc.isEmpty || !s.bcc.isEmpty

        companion object {
            /** A draft is saved this long after the last edit. */
            const val AUTOSAVE_MS = 2_000L
        }

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
        ) {
            _state.update { s ->
                when (which) {
                    RecipientField.TO -> s.copy(to = change(s.to))
                    RecipientField.CC -> s.copy(cc = change(s.cc))
                    RecipientField.BCC -> s.copy(bcc = change(s.bcc))
                }
            }
            scheduleSave()
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

/** Most recipient suggestions shown at once. */
private const val MAX_SUGGESTIONS = 8

/** Many providers refuse messages over this size (M2-04). */
const val ATTACHMENT_WARNING_BYTES = 25L * 1024 * 1024

/** Whether the attached files add up to more than [ATTACHMENT_WARNING_BYTES]. */
val ComposeUiState.attachmentsTooLarge: Boolean
    get() = attachments.sumOf { it.bytes.size.toLong() } > ATTACHMENT_WARNING_BYTES
