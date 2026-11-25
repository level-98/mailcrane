package app.mailcrane.ui.compose

import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.DraftKind
import app.mailcrane.core.DraftSummary
import app.mailcrane.core.OutboxEntry
import app.mailcrane.core.RecipientInput
import app.mailcrane.core.ReplyKind
import app.mailcrane.core.SendResult
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private class FakeUndoCore(
    val waiting: ComposeDraft?,
) : ComposeCore {
    val saved = mutableListOf<ComposeDraft>()

    override suspend fun cancelPending(outboxId: Long) = waiting

    override suspend fun saveDraft(
        draftId: Long?,
        draft: ComposeDraft,
    ): Long {
        saved += draft
        return 31
    }

    override suspend fun enqueueWithDelay(
        draft: ComposeDraft,
        delaySeconds: Int,
    ): Long = 0

    override suspend fun listAccounts() = emptyList<AccountSummary>()

    override fun parseRecipients(
        text: String,
        whole: Boolean,
    ) = RecipientInput(emptyList(), text)

    override suspend fun prepareReply(
        messageId: Long,
        kind: ReplyKind,
        quoteHeader: String,
    ): ComposeDraft = error("unused")

    override suspend fun signatureFor(accountId: Long): String? = null

    override fun applySignature(
        body: String,
        signature: String?,
        kind: DraftKind,
    ) = body

    override fun swapSignature(
        body: String,
        old: String?,
        new: String?,
    ) = body

    override suspend fun getDraft(draftId: Long): ComposeDraft = error("unused")

    override suspend fun openDraft(messageId: Long): Long = 0

    override suspend fun deleteDraft(draftId: Long) = Unit

    override suspend fun listDrafts(accountId: Long) = emptyList<DraftSummary>()

    override suspend fun listOutbox(accountId: Long) = emptyList<OutboxEntry>()

    override suspend fun retryOutbox(outboxId: Long) = Unit

    override suspend fun deleteOutbox(outboxId: Long) = Unit

    override suspend fun editOutbox(outboxId: Long): Long = 0

    override suspend fun send(draft: ComposeDraft) = SendResult(emptyList())
}

@OptIn(ExperimentalCoroutinesApi::class)
class UndoSendViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun undoReopensTheMessageAsADraftExactlyAsItWas() {
        val waiting = ComposeDraft(1, listOf("bob@example.net"), emptyList(), listOf("c@example.com"), "Oops", "Body")
        val core = FakeUndoCore(waiting)
        var opened: Long? = null
        UndoSendViewModel(core).undo(77, onDraft = { opened = it }, onTooLate = {})
        assertEquals(31L, opened)
        assertEquals(listOf(waiting), core.saved)
    }

    @Test
    fun tooLateSaysSo() {
        var late = false
        UndoSendViewModel(FakeUndoCore(null)).undo(77, onDraft = {}, onTooLate = { late = true })
        assertTrue(late)
    }
}
