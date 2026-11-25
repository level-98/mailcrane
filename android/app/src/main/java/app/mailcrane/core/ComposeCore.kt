package app.mailcrane.core

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
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ComposeCore {
    override suspend fun listAccounts(): List<AccountSummary> = withContext(io) { listAccounts(dbPath) }

    override fun parseRecipients(
        text: String,
        whole: Boolean,
    ): RecipientInput = app.mailcrane.core.parseRecipients(text, whole)

    override suspend fun prepareReply(
        messageId: Long,
        kind: ReplyKind,
        quoteHeader: String,
    ): ComposeDraft = withContext(io) { app.mailcrane.core.prepareReply(dbPath, messageId, kind, quoteHeader) }

    override suspend fun send(draft: ComposeDraft): SendResult = app.mailcrane.core.send(dbPath, draft)
}
