package app.mailcrane.ui.compose

import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.DraftFailure
import app.mailcrane.core.DraftKind
import app.mailcrane.core.DraftSummary
import app.mailcrane.core.OutboxEntry
import app.mailcrane.core.RecipientEntry
import app.mailcrane.core.RecipientError
import app.mailcrane.core.RecipientField
import app.mailcrane.core.RecipientInput
import app.mailcrane.core.RecipientProblem
import app.mailcrane.core.ReplyFailure
import app.mailcrane.core.ReplyKind
import app.mailcrane.core.SendFailure
import app.mailcrane.core.SendResult
import app.mailcrane.settings.FakeAppSettings
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ComposeViewModelTest {
    /** Splits at commas like the core does (without quotes); an entry is valid if it has one `@`. */
    private class FakeCore(
        var accounts: List<AccountSummary> = listOf(AccountSummary(1, "ann@example.org", "Ann")),
    ) : ComposeCore {
        var sent: ComposeDraft? = null
        var outcome: suspend () -> SendResult = { SendResult(emptyList()) }

        override suspend fun listAccounts() = accounts

        override fun parseRecipients(
            text: String,
            whole: Boolean,
        ): RecipientInput {
            val parts = text.split(',')
            val done = if (whole) parts else parts.dropLast(1)
            val entries =
                done.map { it.trim() }.filter { it.isNotEmpty() }.map {
                    val ok = it.count { c -> c == '@' } == 1 && !it.startsWith('@') && !it.endsWith('@')
                    RecipientEntry(
                        text = it,
                        name = "",
                        email = if (ok) it else "",
                        problem = if (ok) null else RecipientProblem.MISSING_AT,
                    )
                }
            return RecipientInput(entries, if (whole) "" else parts.last().trimStart())
        }

        override suspend fun send(draft: ComposeDraft): SendResult {
            sent = draft
            return outcome()
        }

        var replies = 0
        var replyFailure: ReplyFailure? = null
        var signatures = mapOf<Long, String>()
        val drafts = mutableMapOf<Long, ComposeDraft>()
        val deleted = mutableListOf<Long>()
        var nextDraftId = 100L

        override suspend fun saveDraft(
            draftId: Long?,
            draft: ComposeDraft,
        ): Long {
            val id = draftId ?: nextDraftId++
            drafts[id] = draft
            return id
        }

        override suspend fun getDraft(draftId: Long): ComposeDraft = drafts[draftId] ?: throw DraftFailure.NotFound()

        override suspend fun openDraft(messageId: Long): Long {
            drafts[7] =
                ComposeDraft(1, listOf("bob@example.net"), emptyList(), listOf("carol@example.com"), "Old", "Body")
            return 7
        }

        override suspend fun deleteDraft(draftId: Long) {
            deleted += draftId
            drafts.remove(draftId)
        }

        override suspend fun listDrafts(accountId: Long): List<DraftSummary> = emptyList()

        val delayed = mutableListOf<Pair<ComposeDraft, Int>>()

        override suspend fun enqueueWithDelay(
            draft: ComposeDraft,
            delaySeconds: Int,
        ): Long {
            delayed += draft to delaySeconds
            return 77
        }

        override suspend fun cancelPending(outboxId: Long): ComposeDraft? = null

        override suspend fun listOutbox(accountId: Long): List<OutboxEntry> = emptyList()

        override suspend fun retryOutbox(outboxId: Long) = Unit

        override suspend fun deleteOutbox(outboxId: Long) = Unit

        override suspend fun editOutbox(outboxId: Long): Long = 0

        override suspend fun signatureFor(accountId: Long): String? = signatures[accountId]

        // Simple stand-ins for the core's rules (tested in Rust).
        override fun applySignature(
            body: String,
            signature: String?,
            kind: DraftKind,
        ): String =
            when {
                signature.isNullOrBlank() -> body
                kind == DraftKind.NEW -> "$body\n\n-- \n$signature"
                else -> "\n\n-- \n$signature\n$body"
            }

        override fun swapSignature(
            body: String,
            old: String?,
            new: String?,
        ): String {
            val oldBlock = old?.let { "\n\n-- \n$it" }
            val newBlock = new?.let { "\n\n-- \n$it" }.orEmpty()
            return when {
                oldBlock == null -> if (body.isEmpty()) newBlock else body
                body.endsWith(oldBlock) -> body.removeSuffix(oldBlock) + newBlock
                else -> body
            }
        }

        override suspend fun prepareReply(
            messageId: Long,
            kind: ReplyKind,
            quoteHeader: String,
        ): ComposeDraft {
            replies++
            replyFailure?.let { throw it }
            return ComposeDraft(
                accountId = 2,
                to = listOf("ana@example.net"),
                cc = if (kind == ReplyKind.REPLY_ALL) listOf("bob@example.com") else emptyList(),
                bcc = emptyList(),
                subject = "Re: Lunch",
                bodyText = "\n\n$quoteHeader\n> Noon?\n",
                inReplyTo = "m2@example.net",
                references = listOf("m1@example.net", "m2@example.net"),
            )
        }
    }

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    private fun started(core: FakeCore = FakeCore()) = ComposeViewModel(core, FakeAppSettings()).apply { start(1) }

    @Test
    fun sendStaysDisabledUntilThereIsAValidRecipient() {
        val vm = started()
        assertFalse(vm.state.value.canSend)
        vm.onSubjectChange("Hi")
        vm.onBodyChange("Text")
        assertFalse(vm.state.value.canSend)
        vm.onRecipientInput(RecipientField.TO, "bob")
        assertFalse("typing an incomplete address", vm.state.value.canSend)
        vm.onRecipientInput(RecipientField.TO, "nope,")
        assertFalse("an invalid chip alone", vm.state.value.canSend)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        assertTrue("a valid address still being typed counts", vm.state.value.canSend)
        vm.onRecipientInput(RecipientField.TO, "")
        assertFalse(vm.state.value.canSend)
        vm.onRecipientInput(RecipientField.CC, "bob@example.net,")
        assertTrue("a valid Cc chip", vm.state.value.canSend)
    }

    @Test
    fun finishedEntriesBecomeChipsAndInvalidOnesAreMarked() {
        val vm = started()
        vm.onRecipientInput(RecipientField.TO, "a@example.org, nope, c@ex")
        val to = vm.state.value.to
        assertEquals(listOf("a@example.org", "nope"), to.chips.map { it.text })
        assertTrue(to.chips[0].isValid)
        assertEquals(RecipientProblem.MISSING_AT, to.chips[1].problem)
        assertEquals("c@ex", to.input)

        vm.commitRecipient(RecipientField.TO)
        assertEquals(3, vm.state.value.to.chips.size)
        assertEquals("", vm.state.value.to.input)

        vm.removeRecipient(RecipientField.TO, 1)
        assertEquals(
            listOf("a@example.org", "c@ex"),
            vm.state.value.to.chips
                .map { it.text },
        )
        vm.removeRecipient(RecipientField.TO, 9)
        assertEquals(2, vm.state.value.to.chips.size)

        vm.editLastRecipient(RecipientField.TO)
        assertEquals("c@ex", vm.state.value.to.input)
        assertEquals(1, vm.state.value.to.chips.size)
    }

    @Test
    fun sendsTheDraftAsTypedAndReportsSent() {
        val core = FakeCore()
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net,")
        vm.showCcBcc()
        vm.onRecipientInput(RecipientField.BCC, "carol@example.com")
        vm.onSubjectChange("Lunch")
        vm.onBodyChange("Noon?")
        vm.send()
        assertEquals(
            ComposeDraft(1, listOf("bob@example.net"), emptyList(), listOf("carol@example.com"), "Lunch", "Noon?"),
            core.sent,
        )
        assertEquals(ComposeResult.Sent(emptyList()), vm.state.value.result)
        vm.reset()
        assertFalse(vm.state.value.hasContent)
        assertNull(vm.state.value.result)
    }

    @Test
    fun invalidChipsBlockSendingAndShowTheError() {
        val core = FakeCore()
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net, nope,")
        vm.send()
        assertNull(core.sent)
        assertEquals(ComposeError.INVALID_RECIPIENTS, vm.state.value.error)
    }

    @Test
    fun failuresKeepTheContent() {
        val core = FakeCore()
        core.outcome = { throw SendFailure.CannotConnect() }
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.onSubjectChange("Lunch")
        vm.onBodyChange("Noon?")
        vm.send()
        val s = vm.state.value
        assertEquals(ComposeError.CANNOT_CONNECT, s.error)
        assertFalse(s.sending)
        assertNull(s.result)
        assertEquals("Lunch", s.subject)
        assertEquals("Noon?", s.body)
        assertEquals(listOf("bob@example.net"), s.to.chips.map { it.text })
        assertTrue("can try again", s.canSend)

        core.outcome = { throw SendFailure.RecipientsRejected(listOf("bob@example.net")) }
        vm.send()
        assertEquals(ComposeError.ALL_REJECTED, vm.state.value.error)
        assertEquals(listOf("bob@example.net"), vm.state.value.errorAddresses)

        core.outcome = { throw SendFailure.Refused(true) }
        vm.send()
        assertEquals(ComposeError.REFUSED_TEMPORARY, vm.state.value.error)

        core.outcome = {
            throw SendFailure.InvalidRecipients(
                listOf(RecipientError(RecipientField.TO, 0u, RecipientProblem.INVALID_DOMAIN)),
            )
        }
        vm.send()
        assertEquals(
            RecipientProblem.INVALID_DOMAIN,
            vm.state.value.to.chips[0]
                .problem,
        )
        assertEquals("Noon?", vm.state.value.body)
    }

    @Test
    fun sendingTwiceAtOnceSendsOnce() {
        val core = FakeCore()
        val gate = CompletableDeferred<SendResult>()
        var calls = 0
        core.outcome = {
            calls++
            gate.await()
        }
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.send()
        assertTrue(vm.state.value.sending)
        assertFalse(vm.state.value.canSend)
        vm.send()
        vm.requestClose()
        assertFalse("can't close while sending", vm.state.value.confirmingDiscard)
        gate.complete(SendResult(listOf("x@example.org")))
        assertEquals(1, calls)
        assertEquals(ComposeResult.Sent(listOf("x@example.org")), vm.state.value.result)
    }

    @Test
    fun closingKeepsWhatWasTypedAsADraft() {
        val core = FakeCore()
        val vm = started(core)
        vm.requestClose()
        assertEquals("nothing typed closes at once", ComposeResult.Closed, vm.state.value.result)
        assertTrue(core.drafts.isEmpty())

        vm.reset()
        vm.start(1)
        vm.onRecipientInput(RecipientField.TO, "half-typed")
        vm.onBodyChange("Draft text")
        vm.requestClose()
        assertEquals(ComposeResult.SavedDraft, vm.state.value.result)
        val saved = core.drafts.values.single()
        assertEquals("Draft text", saved.bodyText)
        assertEquals("a half-typed recipient is kept", listOf("half-typed"), saved.to)
    }

    @Test
    fun aDraftThatCantBeSavedIsNotClosedSilently() {
        val core =
            object : ComposeCore by FakeCore() {
                override suspend fun saveDraft(
                    draftId: Long?,
                    draft: ComposeDraft,
                ): Long = throw DraftFailure.Storage()
            }
        val vm = ComposeViewModel(core, FakeAppSettings()).apply { start(1) }
        vm.onBodyChange("Precious")
        vm.requestClose()
        assertNull(vm.state.value.result)
        assertTrue(vm.state.value.confirmingDiscard)
        assertEquals("Precious", vm.state.value.body)
    }

    @Test
    fun discardAsksAndDeletesTheSavedDraft() {
        val core = FakeCore()
        val vm = started(core)
        vm.onBodyChange("Draft text")
        vm.requestClose()
        val id = core.drafts.keys.single()

        val reopened = ComposeViewModel(core, FakeAppSettings()).apply { start(1, draft = DraftRequest(draftId = id)) }
        reopened.requestDiscard()
        assertTrue(reopened.state.value.confirmingDiscard)
        reopened.keepEditing()
        assertFalse(reopened.state.value.confirmingDiscard)
        assertEquals("Draft text", reopened.state.value.body)
        reopened.requestDiscard()
        reopened.confirmDiscard()
        assertEquals(ComposeResult.Closed, reopened.state.value.result)
        assertEquals(listOf(id), core.deleted)
    }

    @Test
    fun editsAreSavedTwoSecondsAfterTheLastOne() {
        val scheduler = TestCoroutineScheduler()
        Dispatchers.setMain(UnconfinedTestDispatcher(scheduler))
        val core = FakeCore()
        val vm = started(core)
        vm.onBodyChange("H")
        scheduler.advanceTimeBy(1_500)
        vm.onBodyChange("Hi")
        scheduler.advanceTimeBy(1_500)
        scheduler.runCurrent()
        assertTrue("not yet: the timer restarts on each edit", core.drafts.isEmpty())
        scheduler.advanceTimeBy(600)
        scheduler.runCurrent()
        assertEquals(
            "Hi",
            core.drafts.values
                .single()
                .bodyText,
        )
        val id = vm.state.value.draftId
        vm.onBodyChange("Hi there")
        scheduler.advanceTimeBy(2_100)
        scheduler.runCurrent()
        assertEquals("the same draft is updated", id, vm.state.value.draftId)
        assertEquals(1, core.drafts.size)
    }

    @Test
    fun sendingDeletesTheDraft() {
        val core = FakeCore()
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.requestClose()
        val id = core.drafts.keys.single()
        val again = ComposeViewModel(core, FakeAppSettings()).apply { start(1, draft = DraftRequest(draftId = id)) }
        again.send()
        assertEquals(ComposeResult.Sent(emptyList()), again.state.value.result)
        assertEquals(listOf(id), core.deleted)
    }

    @Test
    fun aDraftsFolderMessageOpensAsADraft() {
        val core = FakeCore()
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, draft = DraftRequest(messageId = 55))
        val state = vm.state.value
        assertEquals(7L, state.draftId)
        assertEquals(listOf("bob@example.net"), state.to.chips.map { it.email })
        assertEquals(listOf("carol@example.com"), state.bcc.chips.map { it.email })
        assertTrue(state.showCcBcc)
        assertEquals("Old", state.subject)
        assertEquals("Body", state.body)
        // Rotation doesn't reload over the user's typing.
        vm.onBodyChange("Edited")
        vm.start(1, draft = DraftRequest(messageId = 55))
        assertEquals("Edited", vm.state.value.body)
    }

    @Test
    fun fromPickerOnlyWithSeveralAccountsAndKeepsTheChoice() {
        val core = FakeCore()
        val vm = started(core)
        assertFalse(vm.state.value.showFromPicker)
        assertEquals(1L, vm.state.value.fromAccountId)

        core.accounts = listOf(AccountSummary(1, "ann@example.org", "Ann"), AccountSummary(2, "ann@example.net", ""))
        val vm2 = ComposeViewModel(core, FakeAppSettings()).apply { start(2) }
        assertTrue(vm2.state.value.showFromPicker)
        assertEquals(2L, vm2.state.value.fromAccountId)
        vm2.onFromChange(1)
        vm2.start(2)
        assertEquals("starting again (rotation) keeps the choice", 1L, vm2.state.value.fromAccountId)

        val vm3 = ComposeViewModel(core, FakeAppSettings()).apply { start(99) }
        assertEquals("an unknown account falls back to the first", 1L, vm3.state.value.fromAccountId)
    }

    @Test
    fun replyIsPrefilledOnceAndSendsTheThreadHeaders() {
        val accounts = listOf(AccountSummary(1, "a@example.org", "A"), AccountSummary(2, "b@example.org", "B"))
        val core = FakeCore(accounts = accounts)
        val vm = ComposeViewModel(core, FakeAppSettings())
        val request = ReplyRequest(42, ReplyKind.REPLY_ALL, "Ana wrote:")
        vm.start(1, request)
        val state = vm.state.value
        assertEquals(2L, state.fromAccountId)
        assertEquals(listOf("ana@example.net"), state.to.chips.map { it.email })
        assertEquals(listOf("bob@example.com"), state.cc.chips.map { it.email })
        assertTrue(state.showCcBcc)
        assertEquals("Re: Lunch", state.subject)
        assertEquals("\n\nAna wrote:\n> Noon?\n", state.body)

        // Rotation calls start again: what the user typed is kept.
        vm.onBodyChange("Yes!" + state.body)
        vm.start(1, request)
        assertEquals(1, core.replies)
        assertTrue(
            vm.state.value.body
                .startsWith("Yes!"),
        )

        vm.send()
        val sent = core.sent
        assertEquals("m2@example.net", sent?.inReplyTo)
        assertEquals(listOf("m1@example.net", "m2@example.net"), sent?.references)
        assertEquals(2L, sent?.accountId)
    }

    @Test
    fun aNewMessageHasNoThreadHeaders() {
        val core = FakeCore()
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.send()
        assertNull(core.sent?.inReplyTo)
        assertTrue(
            core.sent
                ?.references
                .orEmpty()
                .isEmpty(),
        )
        assertEquals(0, core.replies)
    }

    @Test
    fun aReplyThatCantBeLoadedLeavesAnEmptyUsableScreen() {
        val core = FakeCore().apply { replyFailure = ReplyFailure.BodyNotDownloaded() }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "x"))
        assertEquals(ComposeError.REPLY_UNAVAILABLE, vm.state.value.error)
        assertFalse(vm.state.value.hasContent)
        assertEquals(1L, vm.state.value.fromAccountId)
    }

    @Test
    fun aNewMessageGetsTheSignatureOnce() {
        val core = FakeCore().apply { signatures = mapOf(1L to "Ann") }
        val vm = started(core)
        assertEquals("\n\n-- \nAnn", vm.state.value.body)
        // Rotation starts the screen again: no second signature.
        vm.start(1)
        assertEquals("\n\n-- \nAnn", vm.state.value.body)
    }

    @Test
    fun aReplyGetsTheSignatureAboveTheQuote() {
        val core = FakeCore().apply { signatures = mapOf(2L to "Bea") }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        assertEquals("\n\n-- \nBea\n\n\nAna wrote:\n> Noon?\n", vm.state.value.body)
    }

    @Test
    fun changingFromSwapsAnUneditedSignatureOnly() {
        val accounts = listOf(AccountSummary(1, "a@example.org", "A"), AccountSummary(2, "b@example.org", "B"))
        val core = FakeCore(accounts).apply { signatures = mapOf(1L to "Ann", 2L to "Bea") }
        val vm = started(core)
        vm.onFromChange(2)
        assertEquals("\n\n-- \nBea", vm.state.value.body)
        vm.onFromChange(1)
        assertEquals("\n\n-- \nAnn", vm.state.value.body)

        vm.onBodyChange(vm.state.value.body + " (on holiday)")
        vm.onFromChange(2)
        assertEquals("\n\n-- \nAnn (on holiday)", vm.state.value.body)
    }

    @Test
    fun noSignatureMeansNoSeparator() {
        val vm = started(FakeCore())
        assertEquals("", vm.state.value.body)
        assertFalse(vm.state.value.hasContent)
    }

    @Test
    fun aQueuedMessageClosesAsQueued() {
        val core = FakeCore().apply { outcome = { SendResult(emptyList(), queued = true) } }
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.send()
        assertEquals(ComposeResult.Queued, vm.state.value.result)
    }

    @Test
    fun withUndoSendTheMessageWaitsAndCanBeTakenBack() {
        val core = FakeCore()
        val vm = ComposeViewModel(core, FakeAppSettings(undoSendSeconds = 10)).apply { start(1) }
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.send()
        assertEquals(ComposeResult.Undoable(77, 10), vm.state.value.result)
        assertEquals(10, core.delayed.single().second)
        assertNull("not sent directly", core.sent)
    }
}
