package app.mailcrane.ui.compose

import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.DraftKind
import app.mailcrane.core.DraftSummary
import app.mailcrane.core.OutboxEntry
import app.mailcrane.core.OutboxEntryState
import app.mailcrane.core.OutboxFailure
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

private fun entry(
    id: Long,
    state: OutboxEntryState,
    error: String? = null,
    attempts: UInt = 0u,
) = OutboxEntry(id, "bob@example.net", "Subject $id", state, error, attempts, 0)

private class FakeOutboxCore : ComposeCore {
    val entries = mutableListOf(entry(1, OutboxEntryState.FAILED, "refused", 1u), entry(2, OutboxEntryState.QUEUED))
    val calls = mutableListOf<String>()

    override suspend fun listOutbox(accountId: Long) = entries.toList()

    override suspend fun retryOutbox(outboxId: Long) {
        calls += "retry $outboxId"
    }

    override suspend fun deleteOutbox(outboxId: Long) {
        if (outboxId == 99L) throw OutboxFailure.NotFound()
        entries.removeAll { it.id == outboxId }
    }

    override suspend fun editOutbox(outboxId: Long): Long {
        entries.removeAll { it.id == outboxId }
        return 500 + outboxId
    }

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

    override suspend fun saveDraft(
        draftId: Long?,
        draft: ComposeDraft,
    ): Long = 0

    override suspend fun getDraft(draftId: Long): ComposeDraft = error("unused")

    override suspend fun openDraft(messageId: Long): Long = 0

    override suspend fun deleteDraft(draftId: Long) = Unit

    override suspend fun listDrafts(accountId: Long) = emptyList<DraftSummary>()

    override suspend fun send(draft: ComposeDraft) = SendResult(emptyList())

    override suspend fun enqueueWithDelay(
        draft: ComposeDraft,
        delaySeconds: Int,
    ): Long = 0

    override suspend fun cancelPending(outboxId: Long): ComposeDraft? = null
}

@OptIn(ExperimentalCoroutinesApi::class)
class OutboxViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun listsRetriesEditsAndDeletes() {
        val core = FakeOutboxCore()
        val vm = OutboxViewModel(core, 1)
        vm.refresh()
        assertEquals(listOf(1L, 2L), vm.entries.value.map { it.id })

        vm.retry(1)
        assertEquals(listOf("retry 1"), core.calls)

        var draft: Long? = null
        vm.edit(1) { draft = it }
        assertEquals(501L, draft)
        assertEquals(listOf(2L), vm.entries.value.map { it.id })

        vm.delete(99) // busy or gone: no crash, the list stays true
        vm.delete(2)
        assertTrue(vm.entries.value.isEmpty())
    }

    @Test
    fun statusTextExplainsEachState() {
        assertEquals(app.mailcrane.R.string.outbox_waiting, outboxStatus(entry(1, OutboxEntryState.QUEUED)))
        assertEquals(
            app.mailcrane.R.string.outbox_will_retry,
            outboxStatus(entry(1, OutboxEntryState.QUEUED, "network", 2u)),
        )
        assertEquals(app.mailcrane.R.string.outbox_sending, outboxStatus(entry(1, OutboxEntryState.SENDING)))
        assertEquals(
            app.mailcrane.R.string.outbox_failed_auth,
            outboxStatus(entry(1, OutboxEntryState.FAILED, "auth")),
        )
        assertEquals(app.mailcrane.R.string.outbox_failed, outboxStatus(entry(1, OutboxEntryState.FAILED, "refused")))
        assertEquals(app.mailcrane.R.string.outbox_not_saved, outboxStatus(entry(1, OutboxEntryState.NOT_SAVED)))
    }
}
