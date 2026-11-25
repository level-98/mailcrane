package app.mailcrane.ui.compose

import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ComposeCore
import app.mailcrane.core.ComposeDraft
import app.mailcrane.core.RecipientEntry
import app.mailcrane.core.RecipientError
import app.mailcrane.core.RecipientField
import app.mailcrane.core.RecipientInput
import app.mailcrane.core.RecipientProblem
import app.mailcrane.core.ReplyFailure
import app.mailcrane.core.ReplyKind
import app.mailcrane.core.SendFailure
import app.mailcrane.core.SendResult
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
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

    private fun started(core: FakeCore = FakeCore()) = ComposeViewModel(core).apply { start(1) }

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
    fun closingWithContentAsksFirst() {
        val vm = started()
        vm.requestClose()
        assertEquals("nothing typed closes at once", ComposeResult.Closed, vm.state.value.result)

        vm.reset()
        vm.onBodyChange("Draft text")
        vm.requestClose()
        assertTrue(vm.state.value.confirmingDiscard)
        assertNull(vm.state.value.result)
        vm.keepEditing()
        assertFalse(vm.state.value.confirmingDiscard)
        assertEquals("Draft text", vm.state.value.body)

        vm.requestClose()
        vm.confirmDiscard()
        assertEquals(ComposeResult.Closed, vm.state.value.result)

        vm.reset()
        vm.onRecipientInput(RecipientField.TO, "half-typed")
        vm.requestClose()
        assertTrue("a half-typed recipient counts as content", vm.state.value.confirmingDiscard)
    }

    @Test
    fun fromPickerOnlyWithSeveralAccountsAndKeepsTheChoice() {
        val core = FakeCore()
        val vm = started(core)
        assertFalse(vm.state.value.showFromPicker)
        assertEquals(1L, vm.state.value.fromAccountId)

        core.accounts = listOf(AccountSummary(1, "ann@example.org", "Ann"), AccountSummary(2, "ann@example.net", ""))
        val vm2 = ComposeViewModel(core).apply { start(2) }
        assertTrue(vm2.state.value.showFromPicker)
        assertEquals(2L, vm2.state.value.fromAccountId)
        vm2.onFromChange(1)
        vm2.start(2)
        assertEquals("starting again (rotation) keeps the choice", 1L, vm2.state.value.fromAccountId)

        val vm3 = ComposeViewModel(core).apply { start(99) }
        assertEquals("an unknown account falls back to the first", 1L, vm3.state.value.fromAccountId)
    }

    @Test
    fun replyIsPrefilledOnceAndSendsTheThreadHeaders() {
        val accounts = listOf(AccountSummary(1, "a@example.org", "A"), AccountSummary(2, "b@example.org", "B"))
        val core = FakeCore(accounts = accounts)
        val vm = ComposeViewModel(core)
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
        val vm = ComposeViewModel(core)
        vm.start(1, ReplyRequest(42, ReplyKind.REPLY, "x"))
        assertEquals(ComposeError.REPLY_UNAVAILABLE, vm.state.value.error)
        assertFalse(vm.state.value.hasContent)
        assertEquals(1L, vm.state.value.fromAccountId)
    }
}
