package app.mailcrane.core

import app.mailcrane.work.OutboxSender
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Writing and sending mail through the core. An interface so view models can be tested with a fake.
 * Address checks and message building all happen in the core.
 */
interface ComposeCore {
    /** The accounts the user can send from, oldest first. */
    suspend fun listAccounts(): List<AccountSummary>

    /** Past correspondents matching [prefix], best first (M2-08). Empty if the database fails. */
    suspend fun suggestContacts(
        accountId: Long?,
        prefix: String,
        limit: Int,
    ): List<ContactSuggestion> = emptyList()

    /**
     * Splits typed recipient text into chips. Without [whole], text after the last comma or semicolon
     * comes back in [RecipientInput.rest] because the user is still typing it.
     */
    fun parseRecipients(
        text: String,
        whole: Boolean,
    ): RecipientInput

    /**
     * A reply, reply-all or forward of [messageId], prefilled by the core (recipients, `Re:`/`Fwd:`
     * subject, quoted text under [quoteHeader], thread headers).
     *
     * @throws ReplyFailure when the message or its body isn't available.
     */
    suspend fun prepareReply(
        messageId: Long,
        kind: ReplyKind,
        quoteHeader: String,
    ): ComposeDraft

    /** The account's signature for a new draft: its text when turned on and not empty, else null. */
    suspend fun signatureFor(accountId: Long): String?

    /** Adds [signature] to [body] where [kind] of draft needs it; null or empty adds nothing. */
    fun applySignature(
        body: String,
        signature: String?,
        kind: DraftKind,
    ): String

    /** Swaps an unedited [old] signature in [body] for [new]; an edited one is left alone. */
    fun swapSignature(
        body: String,
        old: String?,
        new: String?,
    ): String

    /** Saves [draft] locally (new when [draftId] is null); returns its id. Uploads happen later. */
    suspend fun saveDraft(
        draftId: Long?,
        draft: ComposeDraft,
    ): Long

    /** The saved draft. @throws DraftFailure */
    suspend fun getDraft(draftId: Long): ComposeDraft

    /** Opens a message from the Drafts folder as a draft (downloading it if needed); returns the draft id. */
    suspend fun openDraft(messageId: Long): Long

    /** Deletes the draft here and, through the action queue, on the server. */
    suspend fun deleteDraft(draftId: Long)

    /** The account's drafts, most recently changed first. */
    suspend fun listDrafts(accountId: Long): List<DraftSummary>

    /** The account's unsent messages. */
    suspend fun listOutbox(accountId: Long): List<OutboxEntry>

    /** Queues a failed message again. @throws OutboxFailure */
    suspend fun retryOutbox(outboxId: Long)

    /** Removes a message from the outbox. @throws OutboxFailure */
    suspend fun deleteOutbox(outboxId: Long)

    /** Makes an outbox message a draft again; returns the draft id. @throws OutboxFailure */
    suspend fun editOutbox(outboxId: Long): Long

    /** Undo send: queues [draft] to go out after [delaySeconds]; returns its outbox id. @throws SendFailure */
    suspend fun enqueueWithDelay(
        draft: ComposeDraft,
        delaySeconds: Int,
    ): Long

    /** Undo send: the draft back if it hasn't started sending, else null ("Already sent"). */
    suspend fun cancelPending(outboxId: Long): ComposeDraft?

    /**
     * Scheduled send: queues [draft] to go out at [sendAt] (Unix seconds); returns its outbox id.
     * @throws SendFailure
     */
    suspend fun scheduleMessage(
        draft: ComposeDraft,
        sendAt: Long,
    ): Long

    /** Moves a scheduled message that hasn't gone out. @throws OutboxFailure */
    suspend fun rescheduleOutbox(
        outboxId: Long,
        sendAt: Long,
    )

    /**
     * Sends [draft] now.
     *
     * @throws SendFailure when nothing was sent.
     */
    suspend fun send(draft: ComposeDraft): SendResult
}

/** Calls the core through the generated UniFFI bindings. [dbPath] is the app's database file. */
class UniffiComposeCore(
    private val dbPath: String,
    private val outbox: OutboxSender? = null,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ComposeCore {
    override suspend fun listAccounts(): List<AccountSummary> = withContext(io) { listAccounts(dbPath) }

    override suspend fun suggestContacts(
        accountId: Long?,
        prefix: String,
        limit: Int,
    ): List<ContactSuggestion> =
        withContext(io) {
            try {
                suggestContacts(dbPath, accountId, prefix, limit.coerceAtLeast(0).toUInt())
            } catch (_: ContactsFailure) {
                emptyList()
            }
        }

    override fun parseRecipients(
        text: String,
        whole: Boolean,
    ): RecipientInput = app.mailcrane.core.parseRecipients(text, whole)

    override suspend fun prepareReply(
        messageId: Long,
        kind: ReplyKind,
        quoteHeader: String,
    ): ComposeDraft = withContext(io) { app.mailcrane.core.prepareReply(dbPath, messageId, kind, quoteHeader) }

    override suspend fun signatureFor(accountId: Long): String? =
        withContext(io) { signatureForDraft(dbPath, accountId) }

    override fun applySignature(
        body: String,
        signature: String?,
        kind: DraftKind,
    ): String = app.mailcrane.core.applySignature(body, signature, kind)

    override fun swapSignature(
        body: String,
        old: String?,
        new: String?,
    ): String = app.mailcrane.core.swapSignature(body, old, new)

    override suspend fun saveDraft(
        draftId: Long?,
        draft: ComposeDraft,
    ): Long = withContext(io) { app.mailcrane.core.saveDraft(dbPath, draftId, draft) }

    override suspend fun getDraft(draftId: Long): ComposeDraft =
        withContext(io) { app.mailcrane.core.getDraft(dbPath, draftId) }

    override suspend fun openDraft(messageId: Long): Long =
        withContext(io) {
            try {
                app.mailcrane.core.openDraft(dbPath, messageId)
            } catch (_: DraftFailure.BodyNotDownloaded) {
                // Download it once, then open it.
                bodies.fetchBody(messageId)
                app.mailcrane.core.openDraft(dbPath, messageId)
            }
        }

    override suspend fun deleteDraft(draftId: Long) =
        withContext(io) { app.mailcrane.core.deleteDraft(dbPath, draftId) }

    override suspend fun listDrafts(accountId: Long): List<DraftSummary> =
        withContext(io) { app.mailcrane.core.listDrafts(dbPath, accountId) }

    // Only for downloading a draft's body before opening it; events aren't needed here.
    private val bodies by lazy {
        SyncEngine(
            dbPath,
            object : CoreEventListener {
                override fun onEvent(event: CoreEvent) = Unit
            },
        )
    }

    /** Through the outbox: sent at once if possible, otherwise queued and retried in the background. */
    override suspend fun send(draft: ComposeDraft): SendResult {
        val result = sendViaOutbox(dbPath, draft)
        // Queued: keep trying. Sent: the sender also saves the copy to the Sent folder.
        outbox?.kick(draft.accountId)
        return result
    }

    override suspend fun enqueueWithDelay(
        draft: ComposeDraft,
        delaySeconds: Int,
    ): Long {
        val delay = delaySeconds.coerceAtLeast(0).toUInt()
        val id = withContext(io) { app.mailcrane.core.enqueueWithDelay(dbPath, draft, delay) }
        // The sender waits until the delay is over, then sends (and keeps going if the app closes).
        outbox?.kick(draft.accountId)
        return id
    }

    override suspend fun cancelPending(outboxId: Long): ComposeDraft? =
        withContext(io) { app.mailcrane.core.cancelPending(dbPath, outboxId) }

    override suspend fun scheduleMessage(
        draft: ComposeDraft,
        sendAt: Long,
    ): Long {
        val id = withContext(io) { app.mailcrane.core.scheduleMessage(dbPath, draft, sendAt) }
        // Lets the server take it now if it can; the job at the time is the device's fallback.
        outbox?.kick(draft.accountId)
        outbox?.sendAt(sendAt)
        return id
    }

    override suspend fun rescheduleOutbox(
        outboxId: Long,
        sendAt: Long,
    ) {
        withContext(io) { app.mailcrane.core.rescheduleOutbox(dbPath, outboxId, sendAt) }
        outbox?.kickAll()
        outbox?.sendAt(sendAt)
    }

    override suspend fun listOutbox(accountId: Long): List<OutboxEntry> =
        withContext(io) { app.mailcrane.core.listOutbox(dbPath, accountId) }

    override suspend fun retryOutbox(outboxId: Long) {
        withContext(io) { app.mailcrane.core.retryOutbox(dbPath, outboxId) }
        outbox?.kickAll()
    }

    override suspend fun deleteOutbox(outboxId: Long) =
        withContext(io) { app.mailcrane.core.deleteOutbox(dbPath, outboxId) }

    override suspend fun editOutbox(outboxId: Long): Long =
        withContext(io) { app.mailcrane.core.editOutbox(dbPath, outboxId) }
}
