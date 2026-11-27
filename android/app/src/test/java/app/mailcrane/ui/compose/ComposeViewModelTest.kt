package app.mailcrane.ui.compose

import app.mailcrane.contacts.DeviceContacts
import app.mailcrane.contacts.RecipientSuggestion
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeAttachment
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.ContactSuggestion
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
import kotlinx.coroutines.test.StandardTestDispatcher
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

        var contacts: List<ContactSuggestion> = emptyList()

        override suspend fun suggestContacts(
            accountId: Long?,
            prefix: String,
            limit: Int,
        ): List<ContactSuggestion> = contacts.filter { it.email.startsWith(prefix) || it.name.startsWith(prefix, true) }

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

        /** When set, a draft or reply load waits for it, as a slow one would (#435). */
        var loadGate: CompletableDeferred<Unit>? = null
        var signatures = mapOf<Long, String>()

        /** The quote header the last signature swap was given (#449). */
        var lastQuoteHeader: String? = null

        /** A signature lookup for an account in here waits for its gate, as a slow one would (#449). */
        val signatureGates = mutableMapOf<Long, CompletableDeferred<Unit>>()

        /** When set, reading a signature throws it. */
        var signatureFailure: Exception? = null
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

        override suspend fun getDraft(draftId: Long): ComposeDraft {
            loadGate?.await()
            return drafts[draftId] ?: throw DraftFailure.NotFound()
        }

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

        override suspend fun scheduleMessage(
            draft: ComposeDraft,
            sendAt: Long,
        ): Long {
            scheduled += draft to sendAt
            return 88
        }

        override suspend fun rescheduleOutbox(
            outboxId: Long,
            sendAt: Long,
        ) = Unit

        val scheduled = mutableListOf<Pair<ComposeDraft, Long>>()

        override suspend fun cancelPending(outboxId: Long): ComposeDraft? = null

        override suspend fun listOutbox(accountId: Long): List<OutboxEntry> = emptyList()

        override suspend fun retryOutbox(outboxId: Long) = Unit

        override suspend fun deleteOutbox(outboxId: Long) = Unit

        override suspend fun editOutbox(outboxId: Long): Long = 0

        override suspend fun signatureFor(accountId: Long): String? {
            signatureGates[accountId]?.await()
            signatureFailure?.let { throw it }
            return signatures[accountId]
        }

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
            quoteHeader: String?,
            kind: DraftKind?,
        ): String {
            lastQuoteHeader = quoteHeader
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
            loadGate?.await()
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
        // A reply starts writing at the top, above the signature and quote.
        assertFalse(vm.state.value.bodyCursorAtEnd)
    }

    @Test
    fun aReopenedDraftCarriesOnAtItsEnd() {
        // #419: the body field puts the cursor at the end of a saved draft, not at its start.
        val core = FakeCore()
        val vm = started(core)
        assertFalse(vm.state.value.bodyCursorAtEnd)
        vm.onBodyChange("Draft text")
        vm.requestClose()
        val id = core.drafts.keys.single()
        val reopened = ComposeViewModel(core, FakeAppSettings()).apply { start(1, draft = DraftRequest(draftId = id)) }
        assertEquals("Draft text", reopened.state.value.body)
        assertTrue(reopened.state.value.bodyCursorAtEnd)
        // A new message after that starts from scratch.
        reopened.reset()
        assertFalse(reopened.state.value.bodyCursorAtEnd)
    }

    @Test
    fun typingWaitsForADraftToLoad() {
        // #435: edits made while the draft loads are refused; once it lands, typing works and the draft is intact.
        val core = FakeCore()
        val vm = started(core)
        vm.onSubjectChange("Plans")
        vm.onBodyChange("Draft text")
        vm.requestClose()
        val id = core.drafts.keys.single()
        val gate = CompletableDeferred<Unit>()
        core.loadGate = gate
        val reopened = ComposeViewModel(core, FakeAppSettings()).apply { start(1, draft = DraftRequest(draftId = id)) }
        assertTrue(reopened.state.value.loadingContent)
        reopened.onBodyChange("typed too early")
        reopened.onSubjectChange("also too early")
        assertEquals("", reopened.state.value.body)
        assertEquals("", reopened.state.value.subject)
        gate.complete(Unit)
        assertFalse(reopened.state.value.loadingContent)
        assertEquals("Draft text", reopened.state.value.body)
        assertEquals("Plans", reopened.state.value.subject)
        reopened.onBodyChange("Draft text, more")
        assertEquals("Draft text, more", reopened.state.value.body)
    }

    @Test
    fun typingWaitsForAReplyToLoad() {
        val core = FakeCore()
        val gate = CompletableDeferred<Unit>()
        core.loadGate = gate
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        assertTrue(vm.state.value.loadingContent)
        vm.onBodyChange("Thanks")
        assertEquals("", vm.state.value.body)
        gate.complete(Unit)
        assertFalse(vm.state.value.loadingContent)
        assertTrue(
            vm.state.value.body
                .contains("> Noon?"),
        )
        assertEquals("Re: Lunch", vm.state.value.subject)
    }

    @Test
    fun aFailedLoadLeavesTheFieldsEditable() {
        // #435: on failure the fields take typing again, empty, with the existing error.
        val core = FakeCore().apply { replyFailure = ReplyFailure.BodyNotDownloaded() }
        val gate = CompletableDeferred<Unit>()
        core.loadGate = gate
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "x"))
        assertTrue(vm.state.value.loadingContent)
        gate.complete(Unit)
        assertFalse(vm.state.value.loadingContent)
        assertEquals(ComposeError.REPLY_UNAVAILABLE, vm.state.value.error)
        vm.onBodyChange("Typed after all")
        assertEquals("Typed after all", vm.state.value.body)
        // A draft that's gone ends loading too.
        val drafts = FakeCore()
        val reopened = ComposeViewModel(drafts, FakeAppSettings())
        reopened.start(1, draft = DraftRequest(draftId = 999))
        assertFalse(reopened.state.value.loadingContent)
        assertEquals(ComposeError.DRAFT_UNAVAILABLE, reopened.state.value.error)
    }

    @Test
    fun aLoadingFormCantBeSentOrAutosaved() {
        // #435 review: Send waits for the load, and nothing is saved while it runs (that made a second draft).
        val core = FakeCore()
        val gate = CompletableDeferred<Unit>()
        core.loadGate = gate
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        vm.requestClose()
        assertTrue("nothing saved during the load", core.drafts.isEmpty())
        gate.complete(Unit)
        assertTrue(vm.state.value.canSend)
        assertFalse(
            vm.state.value
                .copy(loadingContent = true)
                .canSend,
        )
    }

    @Test
    fun onlyTheNewestLoadUnlocksTheForm() {
        // #435 review: a reset while a load runs cancels it; its end doesn't unlock the next load early.
        val core = FakeCore()
        val first = CompletableDeferred<Unit>()
        core.loadGate = first
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        vm.reset()
        assertFalse("a reset ends the old load", vm.state.value.loadingContent)
        val second = CompletableDeferred<Unit>()
        core.loadGate = second
        vm.start(1, ReplyRequest(43, ReplyKind.REPLY, "Ana wrote:"))
        first.complete(Unit)
        assertTrue("the old load can't unlock the new one", vm.state.value.loadingContent)
        assertEquals("", vm.state.value.body)
        second.complete(Unit)
        assertFalse(vm.state.value.loadingContent)
        assertTrue(
            vm.state.value.body
                .contains("> Noon?"),
        )
    }

    @Test
    fun aSignatureThatCantBeReadDoesntStopTheReply() {
        // #435 review: signatureFor throwing during a reply load crashed the app; the reply goes without one.
        val core = FakeCore().apply { signatureFailure = IllegalStateException("store") }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        assertFalse(vm.state.value.loadingContent)
        assertTrue(
            vm.state.value.body
                .contains("> Noon?"),
        )
        assertFalse(
            "no signature block",
            vm.state.value.body
                .contains("-- \n"),
        )
        assertEquals("Re: Lunch", vm.state.value.subject)
    }

    @Test
    fun aSignatureThatCantBeReadNeverCrashesCompose() {
        // The same failure on a new message and on a From switch: no crash, the body and signature stay.
        val core = FakeCore().apply { signatures = mapOf(1L to "Ann") }
        val vm = started(core)
        assertEquals("\n\n-- \nAnn", vm.state.value.body)
        core.signatureFailure = IllegalStateException("store")
        vm.onFromChange(2)
        assertEquals("\n\n-- \nAnn", vm.state.value.body)
        assertEquals("Ann", vm.state.value.signature)
        val fresh = started(FakeCore().apply { signatureFailure = IllegalStateException("store") })
        assertEquals("", fresh.state.value.body)
    }

    @Test
    fun aHungLoadGivesUpAndUnlocks() {
        val scheduler = TestCoroutineScheduler()
        Dispatchers.setMain(UnconfinedTestDispatcher(scheduler))
        val core = FakeCore().apply { loadGate = CompletableDeferred() }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        scheduler.advanceTimeBy(ComposeViewModel.LOAD_TIMEOUT_MS - 1)
        assertTrue(vm.state.value.loadingContent)
        scheduler.advanceTimeBy(2)
        assertFalse(vm.state.value.loadingContent)
        assertEquals(ComposeError.REPLY_UNAVAILABLE, vm.state.value.error)
    }

    @Test
    fun aRefusedEditTellsTheFieldToTakeTheBodyBack() {
        // #435 review: the field showed the refused keystroke; a new revision makes it re-apply the state.
        val core = FakeCore().apply { loadGate = CompletableDeferred() }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        val before = vm.state.value.bodyRevision
        vm.onBodyChange("x")
        assertTrue(vm.state.value.bodyRevision > before)
        assertEquals("", vm.state.value.body)
        // The subject the same way, merged with #429's revisions.
        val subjectBefore = vm.state.value.subjectRevision
        vm.onSubjectChange("y")
        assertTrue(vm.state.value.subjectRevision > subjectBefore)
        assertEquals("", vm.state.value.subject)
    }

    @Test
    fun onlyOutsideChangesMoveTheSubjectRevision() {
        // #429: the subject field takes the subject over its own text only when this moves.
        val core = FakeCore()
        val vm = started(core)
        val start = vm.state.value.subjectRevision
        vm.onSubjectChange("Lunch", start)
        assertEquals("typing doesn't", start, vm.state.value.subjectRevision)
        assertEquals("Lunch", vm.state.value.subject)
        vm.requestClose()
        val id = core.drafts.keys.single()
        val reopened = ComposeViewModel(core, FakeAppSettings()).apply { start(1, draft = DraftRequest(draftId = id)) }
        assertEquals("Lunch", reopened.state.value.subject)
        assertTrue("a loaded draft does", reopened.state.value.subjectRevision > 0)
        val loaded = reopened.state.value.subjectRevision
        reopened.reset()
        assertTrue("so does a reset", reopened.state.value.subjectRevision > loaded)
        assertEquals("", reopened.state.value.subject)
    }

    @Test
    fun aSubjectTypedBeforeALoadDoesntReplaceTheLoadedOne() {
        // #429: typed against the revision before a reply loaded; the reply's subject stays, and the field is
        // told to take it over.
        val vm = ComposeViewModel(FakeCore(), FakeAppSettings())
        val before = vm.state.value.subjectRevision
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "Ana wrote:"))
        val replied = vm.state.value.subject
        val loaded = vm.state.value.subjectRevision
        assertTrue(loaded > before)
        vm.onSubjectChange("x", before)
        assertEquals(replied, vm.state.value.subject)
        assertTrue(vm.state.value.subjectRevision > loaded)
    }

    @Test
    fun onlyOutsideChangesMoveTheBodyRevision() {
        // #419: the field takes the body over its own text only when this moves.
        val core = FakeCore().apply { signatures = mapOf(1L to "Ann") }
        val vm = started(core)
        val signed = vm.state.value.bodyRevision
        assertTrue("the signature is an outside change", signed > 0)
        vm.onBodyChange("Hi" + vm.state.value.body, signed)
        assertEquals("typing doesn't", signed, vm.state.value.bodyRevision)
        vm.list(0, 0, ListKind.BULLET)
        assertTrue("a list toggle does", vm.state.value.bodyRevision > signed)
        val listed = vm.state.value.bodyRevision
        vm.reset()
        assertTrue("so does a reset", vm.state.value.bodyRevision > listed)
        assertEquals("", vm.state.value.body)
    }

    @Test
    fun anEditTypedAgainstAnOlderBodyIsMovedOntoTheNewOne() {
        // Review of #426: the field typed "H", then "Hi", but the signature landed in between; neither is lost.
        val core = FakeCore()
        val vm = started(core)
        val base = vm.state.value.bodyRevision
        vm.onBodyChange("H", base)
        // An outside change lands (as the signature would) while the field still shows "H".
        vm.list(0, 1, ListKind.BULLET)
        val listed = vm.state.value.body
        assertTrue(listed.endsWith("H"))
        vm.onBodyChange("Hi", base)
        assertEquals(listed.dropLast(1) + "Hi", vm.state.value.body)
        // A second late keystroke against the same old revision builds on the first, not on "H" again.
        vm.onBodyChange("Hi!", base)
        assertEquals(listed.dropLast(1) + "Hi!", vm.state.value.body)
        // The field is told to take the merged text over: a new revision.
        assertTrue(vm.state.value.bodyRevision > base + 1)
    }

    @Test
    fun aLateEditCantRefillABodyClearedFromOutside() {
        val vm = started(FakeCore())
        val base = vm.state.value.bodyRevision
        vm.onBodyChange("abc", base)
        vm.reset()
        vm.onBodyChange("abcd", base)
        assertEquals("", vm.state.value.body)
    }

    @Test
    fun rebaseEditPlacesTheEditAtTheSamePlace() {
        // Typed at the end, a signature added below.
        assertEquals("Hi\n\n-- \nBea", rebaseEdit("H", "Hi", "H\n\n-- \nBea")?.text)
        // Typed before the signature arrived, into an empty body: both kept.
        assertEquals("Hello\n\n-- \nAnn", rebaseEdit("", "Hello", "\n\n-- \nAnn")?.text)
        // A word replaced in the middle, a line added at the top.
        assertEquals("Top\nsee you Friday", rebaseEdit("see you Monday", "see you Friday", "Top\nsee you Monday")?.text)
        // Nothing typed: theirs as it is.
        assertEquals("theirs", rebaseEdit("same", "same", "theirs")?.text)
        // The text around the edit is gone (a reset): can't be placed.
        assertNull(rebaseEdit("abc", "abcd", ""))
        // The text it deleted was changed from outside: can't be placed.
        assertNull(rebaseEdit("one two", "one", "one three"))
        // Typed at the end while something was put in front; typed at the start while something was added after.
        assertEquals("• Hi", rebaseEdit("H", "Hi", "• H")?.text)
        assertEquals("Xabc\n-- \nAnn", rebaseEdit("abc", "Xabc", "abc\n-- \nAnn")?.text)
        // Select all, then paste: the span that still reads as the old text is replaced, the signature kept.
        assertEquals("Dear team\n\n-- \nAnn", rebaseEdit("Hello", "Dear team", "Hello\n\n-- \nAnn")?.text)
        assertNull(rebaseEdit("Hello", "Dear team", "Goodbye"))
    }

    @Test
    fun aRebasedEditPutsTheCursorRightAfterIt() {
        // Review of #426: offsets in the older text would put the cursor before "Hi" in "• Hi".
        assertEquals(Rebased("• Hi", 4), rebaseEdit("H", "Hi", "• H"))
        assertEquals(
            Rebased("Top\nsee you Friday", 15),
            rebaseEdit("see you Monday", "see you Friday", "Top\nsee you Monday"),
        )
        assertEquals(Rebased("Dear team\n-- \nAnn", 9), rebaseEdit("Hello", "Dear team", "Hello\n-- \nAnn"))
        // The view model publishes it with the merged revision; any other outside change clears it.
        val vm = started(FakeCore())
        val base = vm.state.value.bodyRevision
        vm.onBodyChange("H", base)
        vm.list(0, 1, ListKind.BULLET)
        assertNull(vm.state.value.bodyCursor)
        vm.onBodyChange("Hi", base)
        assertEquals(vm.state.value.body.length, vm.state.value.bodyCursor)
        vm.list(0, 0, ListKind.BULLET)
        assertNull(vm.state.value.bodyCursor)
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
    fun typingWhileTheSignatureArrivesKeepsBoth() {
        // #430: the field shows "H" and keeps typing while the signature is added; its edits are based on the
        // revision before the signature. Both the typed text and the signature end up in the body.
        val core = FakeCore().apply { signatures = mapOf(1L to "Ann") }
        val vm = ComposeViewModel(core, FakeAppSettings())
        val typing = vm.state.value.bodyRevision
        vm.onBodyChange("H", typing)
        vm.start(1)
        assertEquals("H\n\n-- \nAnn", vm.state.value.body)
        vm.onBodyChange("Hi", typing)
        assertEquals(2, vm.state.value.bodyCursor)
        vm.onBodyChange("Hi there", typing)
        assertEquals("Hi there\n\n-- \nAnn", vm.state.value.body)
        assertEquals(1, separators(vm.state.value.body))
        // And the field is told to put the cursor after the typing, not where the old offset points.
        assertEquals(8, vm.state.value.bodyCursor)
    }

    @Test
    fun switchingFromWhileTypingKeepsTheTextAndTheNewSignature() {
        // #430: the From swap lands between two keystrokes; the second, typed against the old body, is moved
        // onto the new one. (This fake core swaps by position; the real core's matching of the whole `-- `
        // block by its content is tested in mailcrane-core's compose/signature.rs.)
        val accounts = listOf(AccountSummary(1, "a@example.org", "A"), AccountSummary(2, "b@example.org", "B"))
        val core = FakeCore(accounts).apply { signatures = mapOf(1L to "Ann", 2L to "Bea") }
        val vm = started(core)
        val signed = vm.state.value.bodyRevision
        vm.onBodyChange("Hi\n\n-- \nAnn", signed)
        vm.onFromChange(2)
        assertEquals("Hi\n\n-- \nBea", vm.state.value.body)
        vm.onBodyChange("Hi!\n\n-- \nAnn", signed)
        assertEquals("Hi!\n\n-- \nBea", vm.state.value.body)
        // Typing on against the new revision, then switching back: still one signature, the right one.
        vm.onBodyChange("Hi! See you\n\n-- \nBea", vm.state.value.bodyRevision)
        vm.onFromChange(1)
        assertEquals("Hi! See you\n\n-- \nAnn", vm.state.value.body)
    }

    @Test
    fun repeatedSwitchesNeverDuplicateTheSignature() {
        val accounts = listOf(AccountSummary(1, "a@example.org", "A"), AccountSummary(2, "b@example.org", "B"))
        val core = FakeCore(accounts).apply { signatures = mapOf(1L to "Ann", 2L to "Bea") }
        val vm = started(core)
        vm.onBodyChange("Notes\n\n-- \nAnn", vm.state.value.bodyRevision)
        repeat(5) {
            vm.onFromChange(2)
            assertEquals("Notes\n\n-- \nBea", vm.state.value.body)
            vm.onFromChange(1)
            assertEquals("Notes\n\n-- \nAnn", vm.state.value.body)
        }
        assertEquals(1, separators(vm.state.value.body))
    }

    /** A view model whose coroutines run only when the test says, so lookups can finish in any order (#449). */
    private fun steppedScheduler(): TestCoroutineScheduler {
        val scheduler = TestCoroutineScheduler()
        Dispatchers.setMain(StandardTestDispatcher(scheduler))
        return scheduler
    }

    @Test
    fun aFromSwitchBeforeTheFirstSignatureArrivesGivesOneSignatureForTheNewFrom() {
        // #449 race 1: the first signature (account 1) is still loading when From becomes account 2, and the
        // switch's lookup lands first. The late first one must not add a second block.
        val scheduler = steppedScheduler()
        val first = CompletableDeferred<Unit>()
        val core =
            FakeCore().apply {
                signatures = mapOf(1L to "Ann", 2L to "Bea")
                signatureGates[1L] = first
            }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1)
        scheduler.runCurrent()
        vm.onFromChange(2)
        scheduler.runCurrent()
        assertEquals("\n\n-- \nBea", vm.state.value.body)
        first.complete(Unit)
        scheduler.runCurrent()
        assertEquals("\n\n-- \nBea", vm.state.value.body)
        assertEquals("Bea", vm.state.value.signature)
        assertEquals(1, separators(vm.state.value.body))
    }

    @Test
    fun typingThenSwitchingBeforeTheFirstSignatureArrivesKeepsTheTextAndOneSignature() {
        // #449 race 1, with text typed first: it gets the new From's signature, not the old one's late result.
        val scheduler = steppedScheduler()
        val first = CompletableDeferred<Unit>()
        val core =
            FakeCore().apply {
                signatures = mapOf(1L to "Ann", 2L to "Bea")
                signatureGates[1L] = first
            }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1)
        scheduler.runCurrent()
        vm.onBodyChange("Hi")
        vm.onFromChange(2)
        scheduler.runCurrent()
        first.complete(Unit)
        scheduler.runCurrent()
        assertEquals("Hi\n\n-- \nBea", vm.state.value.body)
        assertEquals("Bea", vm.state.value.signature)
        assertEquals(2L, vm.state.value.fromAccountId)
    }

    @Test
    fun fallingBackFromARemovedAccountGivesTheFallbackAccountsSignature() {
        // #449 review: account 1 is gone by the time the account list loads, so From falls back to account 2.
        // That is a change of From too: account 2's signature goes in, and account 1's late one is dropped.
        val scheduler = steppedScheduler()
        val first = CompletableDeferred<Unit>()
        val core =
            FakeCore().apply {
                accounts = listOf(AccountSummary(2, "bea@example.org", "Bea"))
                signatures = mapOf(1L to "Ann", 2L to "Bea")
                signatureGates[1L] = first
            }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1)
        scheduler.runCurrent()
        assertEquals(2L, vm.state.value.fromAccountId)
        first.complete(Unit)
        scheduler.runCurrent()
        assertEquals("\n\n-- \nBea", vm.state.value.body)
        assertEquals("Bea", vm.state.value.signature)
    }

    @Test
    fun aRepliesFromSwitchOnlyLooksAboveItsQuote() {
        // #449 review: the core is told where the user's own text ends (the quote header), so a forwarded or
        // quoted copy of their signature is never the one swapped.
        val core = FakeCore().apply { signatures = mapOf(1L to "Ann", 2L to "Bea") }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1, ReplyRequest(42, ReplyKind.FORWARD, "Forwarded message:"))
        vm.onFromChange(1)
        assertEquals("Forwarded message:", core.lastQuoteHeader)
    }

    @Test
    fun fastSwitchesApplyOnlyTheNewestLookup() {
        // #449 race 2: From 1 → 2 → 1 quickly; account 1's lookup lands first, then account 2's slow one. The
        // stale one is dropped, so the message keeps account 1's signature, with the typing intact.
        val scheduler = steppedScheduler()
        val core = FakeCore().apply { signatures = mapOf(1L to "Ann", 2L to "Bea") }
        val vm = ComposeViewModel(core, FakeAppSettings())
        vm.start(1)
        scheduler.runCurrent()
        vm.onBodyChange("Hi" + vm.state.value.body)
        assertEquals("Hi\n\n-- \nAnn", vm.state.value.body)
        val slow = CompletableDeferred<Unit>()
        core.signatureGates[2L] = slow
        vm.onFromChange(2)
        vm.onFromChange(1)
        scheduler.runCurrent()
        slow.complete(Unit)
        scheduler.runCurrent()
        assertEquals("Hi\n\n-- \nAnn", vm.state.value.body)
        assertEquals("Ann", vm.state.value.signature)
        assertEquals(1L, vm.state.value.fromAccountId)
        assertEquals(1, separators(vm.state.value.body))
    }

    /** How many signature separator lines the body has. */
    private fun separators(body: String) = body.lines().count { it == "-- " }

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

    @Test
    fun aScheduledMessageIsQueuedForItsTimeAndClosesTheScreen() {
        val core = FakeCore()
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net")
        vm.schedule(1_900_000_000)
        assertEquals(ComposeResult.Scheduled(1_900_000_000), vm.state.value.result)
        assertEquals(1_900_000_000L, core.scheduled.single().second)
        assertNull("not sent now", core.sent)
    }

    private class FakeDevice(
        override val available: Boolean,
        val people: List<RecipientSuggestion> = emptyList(),
    ) : DeviceContacts {
        var searched = 0

        override suspend fun search(
            prefix: String,
            limit: Int,
        ): List<RecipientSuggestion> {
            searched++
            return if (available) people.filter { it.name.startsWith(prefix, true) } else emptyList()
        }
    }

    @Test
    fun typingSuggestsAndPickingAddsAChip() {
        val core = FakeCore().apply { contacts = listOf(ContactSuggestion("ana@example.net", "Ana Silva")) }
        val device = FakeDevice(true, listOf(RecipientSuggestion("Anabel", "anabel@example.net", fromDevice = true)))
        val vm = ComposeViewModel(core, FakeAppSettings(), device).apply { start(1) }
        vm.onRecipientInput(RecipientField.TO, "ana")
        val state = vm.state.value
        assertEquals(RecipientField.TO, state.suggestionsFor)
        assertEquals(listOf("ana@example.net", "anabel@example.net"), state.suggestions.map { it.email })
        assertFalse("allowed already: no question", state.askContacts)
        vm.pickSuggestion(RecipientField.TO, state.suggestions[0])
        val after = vm.state.value
        assertEquals(listOf("\"Ana Silva\" <ana@example.net>"), after.to.chips.map { it.text })
        assertEquals("", after.to.input)
        assertNull(after.suggestionsFor)
    }

    @Test
    fun theContactsQuestionIsAskedOnceAndDecliningKeepsPastCorrespondents() {
        val settings = FakeAppSettings()
        val core = FakeCore().apply { contacts = listOf(ContactSuggestion("bob@example.net", "Bob")) }
        val device = FakeDevice(false)
        val vm = ComposeViewModel(core, settings, device).apply { start(1) }
        vm.onRecipientInput(RecipientField.TO, "b")
        assertTrue(vm.state.value.askContacts)
        assertEquals(
            listOf("bob@example.net"),
            vm.state.value.suggestions
                .map { it.email },
        )
        vm.contactsAnswered()
        assertFalse(vm.state.value.askContacts)
        val again = ComposeViewModel(core, settings, device).apply { start(1) }
        again.onRecipientInput(RecipientField.TO, "b")
        assertFalse("not asked twice", again.state.value.askContacts)
        assertEquals(
            listOf("bob@example.net"),
            again.state.value.suggestions
                .map { it.email },
        )
    }

    @Test
    fun committingOrClearingHidesSuggestions() {
        val core = FakeCore().apply { contacts = listOf(ContactSuggestion("bob@example.net", "Bob")) }
        val vm = ComposeViewModel(core, FakeAppSettings(), FakeDevice(true)).apply { start(1) }
        vm.onRecipientInput(RecipientField.CC, "bo")
        assertEquals(RecipientField.CC, vm.state.value.suggestionsFor)
        vm.onRecipientInput(RecipientField.CC, "")
        assertNull(vm.state.value.suggestionsFor)
        vm.onRecipientInput(RecipientField.CC, "bo")
        vm.commitRecipient(RecipientField.CC)
        assertNull(vm.state.value.suggestionsFor)
    }

    @Test
    fun attachmentsAreAddedRemovedAndSent() {
        val core = FakeCore()
        val vm = started(core)
        vm.addAttachments(
            listOf(
                ComposeAttachment("a.pdf", "application/pdf", byteArrayOf(1, 2, 3)),
                ComposeAttachment("b.png", "image/png", byteArrayOf(4)),
            ),
        )
        vm.removeAttachment(0)
        assertEquals(
            listOf("b.png"),
            vm.state.value.attachments
                .map { it.filename },
        )
        assertFalse(vm.state.value.attachmentsTooLarge)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net,")
        vm.send()
        assertEquals(listOf("b.png"), core.sent?.attachments?.map { it.filename })
    }

    @Test
    fun overTwentyFiveMegabytesWarns() {
        val vm = started()
        vm.addAttachments(listOf(ComposeAttachment("big.bin", "application/octet-stream", ByteArray(26 * 1024 * 1024))))
        assertTrue(vm.state.value.attachmentsTooLarge)
    }

    @Test
    fun formattedBodiesAreSentRichUnlessPlainIsChosen() {
        val core = FakeCore()
        val vm = started(core)
        vm.onRecipientInput(RecipientField.TO, "bob@example.net,")
        vm.onBodyChange("Hello world")
        vm.format(0, 5, InlineFormat.BOLD)
        vm.send()
        assertTrue(core.sent?.rich != null)

        val plainCore = FakeCore()
        val plain = started(plainCore)
        plain.onRecipientInput(RecipientField.TO, "bob@example.net,")
        plain.onBodyChange("Hello world")
        plain.format(0, 5, InlineFormat.BOLD)
        plain.setPlainTextOnly(true)
        plain.send()
        assertNull(plainCore.sent?.rich)

        val unformatted = FakeCore()
        val text = started(unformatted)
        text.onRecipientInput(RecipientField.TO, "bob@example.net,")
        text.onBodyChange("Just text")
        text.send()
        assertNull("no formatting: plain text", unformatted.sent?.rich)
    }
}
