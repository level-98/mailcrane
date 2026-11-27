package app.mailcrane.ui.message

import app.mailcrane.core.ActionFailure
import app.mailcrane.core.AuthResult
import app.mailcrane.core.AuthVerdict
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.InlinePart
import app.mailcrane.core.LinkCheck
import app.mailcrane.core.LinkChecker
import app.mailcrane.core.LinkCleaner
import app.mailcrane.core.LinkText
import app.mailcrane.core.LinkWarning
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageAddress
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageCore
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.SenderCheck
import app.mailcrane.core.SenderTrustChanges
import app.mailcrane.core.SnoozeFailure
import app.mailcrane.core.SnoozeInfo
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.UnsubscribeFailure
import app.mailcrane.core.UnsubscribeInfo
import app.mailcrane.core.UnsubscribeMailto
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.MarkReadMode
import app.mailcrane.ui.compose.ComposePrefill
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private const val ID = 42L

private val NoCleaning = LinkCleaner { it }

/** Stands in for the core: marks the link so the tests can see whether it was cleaned. */
private val MarkingCleaner = LinkCleaner { "cleaned:$it" }

private fun messageViewModel(
    core: MessageCore = FakeMessageCore(),
    settings: FakeAppSettings = FakeAppSettings(),
    cleaner: LinkCleaner = NoCleaning,
    checker: LinkChecker = NoWarnings,
    trust: SenderTrustChanges = SenderTrustChanges(),
) = MessageViewModel(core, settings, cleaner, checker, trust, ID)

private val NoWarnings =
    LinkChecker { _, url, _ -> LinkCheck(url.substringAfter("://").substringBefore('/'), "", false, emptyList()) }

private fun warning(vararg warnings: LinkWarning) =
    LinkChecker { _, _, _ -> LinkCheck("example.net", "example.net", false, warnings.toList()) }

private fun detail(
    body: MessageBody,
    isSeen: Boolean = false,
    isFlagged: Boolean = false,
) = MessageDetail(
    id = ID,
    folderId = 7,
    from = MessageAddress("Ana", "ana@example.org"),
    to = listOf(MessageAddress("", "bob@example.com")),
    cc = emptyList(),
    date = 1_772_445_600,
    subject = "Hi",
    isSeen = isSeen,
    isFlagged = isFlagged,
    body = body,
    attachments = emptyList(),
)

private fun html(
    remoteBlocked: UInt,
    allowRemote: Boolean,
) = MessageBody.Html(
    html = "<p>remote=$allowRemote</p>",
    remoteBlocked = remoteBlocked,
    trackersRemoved = 0u,
    trackers = emptyList(),
    links = HTML_LINKS,
)

private val HTML_LINKS = listOf(LinkText("https://example.net/login", "https://example.com/login"))

private class FakeMessageCore(
    var downloaded: Boolean = true,
    var seen: Boolean = false,
    var remoteBlocked: UInt = 2u,
    var plain: Boolean = false,
) : MessageCore {
    override val events = MutableSharedFlow<CoreEvent>(extraBufferCapacity = 8)

    val calls = mutableListOf<String>()
    var fetchFailure: SyncErrorKind? = null
    var markReadFailure: SyncErrorKind? = null

    /** When set, [markRead] waits for it, so a test can act while the call is still running. */
    var markReadGate: kotlinx.coroutines.CompletableDeferred<Unit>? = null
    var parts = mapOf<String, InlinePart>()

    /** The sender check's verdict to report, or null for no chip (M5-08). */
    var verdict: AuthVerdict? = null

    override suspend fun getMessage(
        messageId: Long,
        allowRemote: Boolean,
    ): MessageDetail {
        assertEquals(ID, messageId)
        calls += "get(remote=$allowRemote)"
        val body =
            when {
                !downloaded -> MessageBody.NotDownloaded
                plain -> MessageBody.Plain("Hello")
                else -> html(if (allowRemote) 0u else remoteBlocked, allowRemote)
            }
        val check =
            verdict?.let {
                SenderCheck(
                    trusted = true,
                    verdict = it,
                    spf = AuthResult.PASS,
                    spfDomain = null,
                    dkim = AuthResult.PASS,
                    dkimDomain = null,
                    dmarc = AuthResult.PASS,
                    dmarcDomain = null,
                )
            }
        return detail(body, seen).copy(senderCheck = check)
    }

    override suspend fun fetchBody(messageId: Long) {
        calls += "fetch"
        fetchFailure?.let { throw SyncFailure.Failed(it) }
        downloaded = true
    }

    override suspend fun markRead(messageId: Long) {
        calls += "markRead"
        markReadGate?.await()
        markReadFailure?.let { throw SyncFailure.Failed(it) }
        seen = true
    }

    override fun inlinePart(
        messageId: Long,
        contentId: String,
    ): InlinePart? = parts[contentId]

    val actions = mutableListOf<MessageAction>()
    var actionFailure: ActionFailure? = null
    var unsubscribe: UnsubscribeInfo? = null
    var unsubscribeFailure: UnsubscribeFailure? = null

    override suspend fun unsubscribeOptions(messageId: Long): UnsubscribeInfo? = unsubscribe

    override suspend fun oneClickUnsubscribe(messageId: Long) {
        calls += "oneClick"
        unsubscribeFailure?.let { throw it }
    }

    var snoozeInfo: SnoozeInfo? = null
    var snoozeFailure: SnoozeFailure? = null

    override suspend fun snooze(
        messageId: Long,
        wakeAt: Long,
    ) {
        assertEquals(ID, messageId)
        snoozeFailure?.let { throw it }
        calls += "snooze($wakeAt)"
    }

    override suspend fun unsnooze(messageId: Long) {
        snoozeFailure?.let { throw it }
        calls += "unsnooze"
    }

    var snoozeInfoFailure: SnoozeFailure? = null

    override suspend fun snoozeInfo(messageId: Long): SnoozeInfo? {
        snoozeInfoFailure?.let { throw it }
        return snoozeInfo
    }

    override suspend fun applyAction(
        messageId: Long,
        action: MessageAction,
    ) {
        assertEquals(ID, messageId)
        actionFailure?.let { throw it }
        actions += action
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class MessageViewModelTest {
    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    private fun ready(vm: MessageViewModel) = vm.state.value as MessageState.Ready

    @Test
    fun aClearedProviderServerNameDropsTheVerdictAtOnce() {
        // M5-08: the setting is saved, then cleared, while this message screen stays alive.
        val core = FakeMessageCore().apply { verdict = AuthVerdict.VERIFIED }
        val trust = SenderTrustChanges()
        val vm = messageViewModel(core = core, trust = trust)
        val shown = { (vm.state.value as MessageState.Ready).message.senderCheck?.verdict }
        assertEquals(AuthVerdict.VERIFIED, shown())

        core.verdict = null
        trust.changed()
        assertNull(shown())
        assertTrue(vm.state.value is MessageState.Ready)

        core.verdict = AuthVerdict.VERIFIED
        trust.changed()
        assertEquals(AuthVerdict.VERIFIED, shown())
    }

    @Test
    fun opens_a_stored_message_with_remote_images_blocked_and_marks_it_read() =
        runTest {
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            val state = ready(vm)
            assertFalse(state.remoteAllowed)
            assertTrue(state.showRemoteBanner)
            assertEquals(listOf("get(remote=false)", "markRead"), core.calls)
        }

    @Test
    fun downloads_the_body_first_when_it_is_not_stored() =
        runTest {
            val core = FakeMessageCore(downloaded = false)
            val vm = messageViewModel(core)
            assertTrue(ready(vm).message.body is MessageBody.Html)
            assertEquals(listOf("get(remote=false)", "fetch", "get(remote=false)", "markRead"), core.calls)
        }

    @Test
    fun a_read_message_is_not_marked_again() =
        runTest {
            val core = FakeMessageCore(seen = true)
            messageViewModel(core)
            assertFalse("markRead" in core.calls)
        }

    @Test
    fun load_remote_images_refetches_with_remote_allowed_for_this_message() =
        runTest {
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            vm.loadRemoteImages()
            val state = ready(vm)
            assertTrue(state.remoteAllowed)
            assertFalse(state.showRemoteBanner)
            assertEquals("<p>remote=true</p>", (state.message.body as MessageBody.Html).html)
            assertEquals(listOf("get(remote=false)", "markRead", "get(remote=true)"), core.calls)

            // A second tap does nothing; a retry keeps the choice.
            vm.loadRemoteImages()
            vm.load()
            assertEquals("get(remote=true)", core.calls.last())

            // Another message starts blocked again.
            val other = messageViewModel(FakeMessageCore())
            assertFalse(ready(other).remoteAllowed)
        }

    @Test
    fun no_banner_when_nothing_was_blocked_or_for_plain_text() =
        runTest {
            assertFalse(ready(messageViewModel(FakeMessageCore(remoteBlocked = 0u))).showRemoteBanner)
            assertFalse(ready(messageViewModel(FakeMessageCore(plain = true))).showRemoteBanner)
        }

    @Test
    fun a_failed_download_shows_the_error_and_retry_recovers() =
        runTest {
            val core = FakeMessageCore(downloaded = false)
            core.fetchFailure = SyncErrorKind.NETWORK
            val vm = messageViewModel(core)
            assertEquals(MessageState.Failed(SyncErrorKind.NETWORK), vm.state.value)
            assertFalse("markRead" in core.calls)

            core.fetchFailure = null
            vm.load()
            assertTrue(vm.state.value is MessageState.Ready)
            assertEquals("markRead", core.calls.last())
        }

    @Test
    fun a_second_tap_while_marking_read_never_shows_it_read_if_marking_fails() =
        runTest {
            // #372 review: the second tap skipped the core and set isSeen while the first call still ran; the
            // first then failed, and the toolbar said "Mark as unread" for an unread message.
            val settings = FakeAppSettings().apply { markRead.value = MarkReadMode.MANUAL }
            val core = FakeMessageCore()
            val vm = messageViewModel(core, settings)
            core.markReadGate = kotlinx.coroutines.CompletableDeferred()
            core.markReadFailure = SyncErrorKind.NETWORK
            vm.markReadByHand()
            vm.markReadByHand()
            core.markReadGate?.complete(Unit)
            assertEquals(1, core.calls.count { it == "markRead" })
            assertFalse("still unread", ready(vm).message.isSeen)
            // And it can be tried again.
            core.markReadFailure = null
            vm.markReadByHand()
            assertTrue(ready(vm).message.isSeen)
        }

    @Test
    fun a_failed_mark_read_still_shows_the_message() =
        runTest {
            val core = FakeMessageCore()
            core.markReadFailure = SyncErrorKind.STORAGE
            val vm = messageViewModel(core)
            assertTrue(vm.state.value is MessageState.Ready)
        }

    @Test
    fun inline_parts_come_from_this_message() =
        runTest {
            val png = InlinePart("image/png", byteArrayOf(1, 2, 3))
            val core = FakeMessageCore().apply { parts = mapOf("logo@example.org" to png) }
            val vm = messageViewModel(core)
            assertEquals(png, vm.inlinePart("logo@example.org"))
            assertNull(vm.inlinePart("other@example.org"))
        }

    @Test
    fun linksAreCleanedWhenTheSettingIsOn() =
        runTest {
            val vm = messageViewModel(settings = FakeAppSettings(cleanLinks = true), cleaner = MarkingCleaner)
            val link = "https://example.com/?utm_source=x"
            assertEquals("cleaned:$link", vm.linkToOpen(link))
        }

    @Test
    fun linksAreLeftAloneWhenTheSettingIsOff() =
        runTest {
            val settings = FakeAppSettings(cleanLinks = false)
            val vm = messageViewModel(settings = settings, cleaner = MarkingCleaner)
            val link = "https://example.com/?utm_source=x"
            assertEquals(link, vm.linkToOpen(link))

            settings.setCleanLinks(true)
            assertEquals("cleaned:https://example.com/", vm.linkToOpen("https://example.com/"))
        }

    @Test
    fun aWebLinkWaitsForConfirmationAndOpensOnlyAfterOpen() =
        runTest {
            val vm = messageViewModel()
            vm.onLinkTapped("https://example.com/a")
            assertEquals("https://example.com/a", vm.linkPreview.value?.url)
            vm.openPreviewedLink()
            assertNull(vm.linkPreview.value)
            assertEquals(MessageEffect.OpenLink("https://example.com/a"), vm.effects.first())
        }

    @Test
    fun cancelOpensNothing() =
        runTest {
            val vm = messageViewModel()
            vm.onLinkTapped("https://example.com/a")
            vm.dismissLinkPreview()
            assertNull(vm.linkPreview.value)
            vm.openPreviewedLink()
            vm.onLinkTapped("mailto:a@example.org")
            // The only effect is the mailto, so nothing opened for the cancelled link.
            assertEquals(MessageEffect.OpenLink("mailto:a@example.org"), vm.effects.first())
        }

    @Test
    fun withTheSettingOffAMatchingLinkOpensAtOnceButAWarningStillAsks() =
        runTest {
            val off = FakeAppSettings(confirmLinks = false)
            val vm = messageViewModel(settings = off)
            vm.onLinkTapped("https://example.com/a")
            assertNull(vm.linkPreview.value)
            assertEquals(MessageEffect.OpenLink("https://example.com/a"), vm.effects.first())

            val warned = messageViewModel(settings = off, checker = warning(LinkWarning.IpAddress))
            warned.onLinkTapped("https://192.0.2.1/")
            assertEquals("https://192.0.2.1/", warned.linkPreview.value?.url)
        }

    @Test
    fun theCleanedLinkIsWhatIsShownAndOpened() =
        runTest {
            val vm = messageViewModel(cleaner = MarkingCleaner)
            vm.onLinkTapped("https://example.com/?utm_source=x")
            assertEquals("cleaned:https://example.com/?utm_source=x", vm.linkPreview.value?.url)
        }

    @Test
    fun plainTextLinksOnlyStopForAHiddenHost() =
        runTest {
            val core = FakeMessageCore(plain = true)
            val calm = messageViewModel(core, checker = warning(LinkWarning.Http))
            calm.onLinkTapped("http://example.com/")
            assertNull(calm.linkPreview.value)
            assertEquals(MessageEffect.OpenLink("http://example.com/"), calm.effects.first())

            val risky = messageViewModel(FakeMessageCore(plain = true), checker = warning(LinkWarning.UserInfo))
            risky.onLinkTapped("https://example.com@example.net/")
            assertEquals("https://example.com@example.net/", risky.linkPreview.value?.url)

            val lookalike =
                messageViewModel(FakeMessageCore(plain = true), checker = warning(LinkWarning.LookalikeHost))
            lookalike.onLinkTapped("https://xn--80ak6aa92e.com/")
            assertEquals("https://xn--80ak6aa92e.com/", lookalike.linkPreview.value?.url)
        }

    @Test
    fun theCoreGetsTheTappedUrlTheCleanedUrlAndTheBodyLinks() =
        runTest {
            val seen = mutableListOf<Triple<String, String, List<LinkText>>>()
            val recording =
                LinkChecker { tapped, target, links ->
                    seen += Triple(tapped, target, links)
                    LinkCheck("example.net", "example.net", false, emptyList())
                }
            val vm = messageViewModel(cleaner = MarkingCleaner, checker = recording)
            ready(vm)
            vm.onLinkTapped("https://example.net/login")
            assertEquals(
                listOf(Triple("https://example.net/login", "cleaned:https://example.net/login", HTML_LINKS)),
                seen,
            )
        }

    @Test
    fun anUnsupportedLinkCanNeverOpen() =
        runTest {
            val vm = messageViewModel(checker = warning(LinkWarning.UnsupportedScheme))
            vm.onLinkTapped("https://example.com/")
            vm.openPreviewedLink()
            vm.onLinkTapped("mailto:a@example.org")
            assertEquals(MessageEffect.OpenLink("mailto:a@example.org"), vm.effects.first())
        }

    @Test
    fun archiveDeleteAndMoveLeaveAndMarkUnreadCloses() =
        runTest {
            // M4-15: leaving the folder hands over to "After deleting or moving"; mark unread just closes.
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            vm.archive()
            assertEquals(MessageEffect.Left, vm.effects.first())
            vm.delete()
            assertEquals(MessageEffect.Left, vm.effects.first())
            vm.moveTo(9)
            assertEquals(MessageEffect.Left, vm.effects.first())
            vm.markUnread()
            assertEquals(MessageEffect.Close, vm.effects.first())
            assertEquals(
                listOf(MessageAction.Archive, MessageAction.Delete, MessageAction.Move(9), MessageAction.MarkUnread),
                core.actions,
            )
        }

    private val mailto = UnsubscribeMailto("unsub@example.net", "stop", null)

    @Test
    fun oneClickUnsubscribeSendsAndRemovesTheButton() =
        runTest {
            val info = UnsubscribeInfo("news@example.net", true, mailto, "https://example.net/u")
            val core = FakeMessageCore().apply { unsubscribe = info }
            val vm = messageViewModel(core)
            assertEquals(info, ready(vm).unsubscribe)
            assertFalse("nothing before the user confirms", core.calls.contains("oneClick"))
            vm.unsubscribe()
            assertEquals(MessageEffect.Unsubscribed(true), vm.effects.first())
            assertTrue(core.calls.contains("oneClick"))
            assertNull(ready(vm).unsubscribe)
        }

    @Test
    fun aFailedOneClickFallsBackToTheOtherWays() =
        runTest {
            val info = UnsubscribeInfo("news@example.net", true, mailto, "https://example.net/u")
            val core =
                FakeMessageCore().apply {
                    unsubscribe = info
                    unsubscribeFailure = UnsubscribeFailure.Refused(302u)
                }
            val vm = messageViewModel(core)
            vm.unsubscribe()
            assertEquals(MessageEffect.Unsubscribed(false), vm.effects.first())
            assertEquals(UnsubscribeMethod.EMAIL, unsubscribeMethod(ready(vm).unsubscribe!!))
            vm.unsubscribe()
            assertEquals(
                MessageEffect.ComposeUnsubscribe(ComposePrefill("unsub@example.net", "stop", "")),
                vm.effects.first(),
            )
        }

    @Test
    fun webOnlyOpensThePage() =
        runTest {
            val core =
                FakeMessageCore().apply {
                    unsubscribe =
                        UnsubscribeInfo("n@example.net", false, null, "https://example.net/u")
                }
            val vm = messageViewModel(core)
            vm.unsubscribe()
            assertEquals(MessageEffect.OpenUnsubscribePage("https://example.net/u"), vm.effects.first())
            assertFalse(core.calls.contains("oneClick"))
        }

    @Test
    fun snoozeAndUnsnoozeCloseTheMessage() =
        runTest {
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            vm.snooze(1_800_000_000)
            assertEquals(MessageEffect.Close, vm.effects.first())
            vm.unsnooze()
            assertEquals(MessageEffect.Close, vm.effects.first())
            assertTrue(core.calls.containsAll(listOf("snooze(1800000000)", "unsnooze")))
        }

    @Test
    fun aSnoozedMessageShowsItsWakeTime() =
        runTest {
            val info = SnoozeInfo(wakeAt = 1_800_000_000, deviceOnly = true, waking = false)
            val core = FakeMessageCore().apply { snoozeInfo = info }
            val vm = messageViewModel(core)
            assertEquals(info, ready(vm).snooze)
        }

    @Test
    fun aMessageWithoutMessageIdCantBeSnoozed() =
        runTest {
            val core = FakeMessageCore().apply { snoozeFailure = SnoozeFailure.NoMessageId() }
            val vm = messageViewModel(core)
            vm.snooze(1_800_000_000)
            assertEquals(MessageEffect.ActionRefused(ActionRefusal.CANNOT_SNOOZE), vm.effects.first())
            assertTrue(vm.state.value is MessageState.Ready)
        }

    /** #431: snoozing again while the message is coming back is refused; it then shows as waking. */
    @Test
    fun aWakingMessageCantBeSnoozedAgainYet() =
        runTest {
            val core = FakeMessageCore().apply { snoozeFailure = SnoozeFailure.Waking() }
            val vm = messageViewModel(core)
            core.snoozeInfo = SnoozeInfo(wakeAt = 1_800_000_000, deviceOnly = false, waking = true)
            vm.snooze(1_900_000_000)
            assertEquals(MessageEffect.ActionRefused(ActionRefusal.WAKING), vm.effects.first())
            assertEquals(true, ready(vm).snooze?.waking)
            // The wake lands: the change re-enables snoozing, with no timer.
            core.snoozeInfo = null
            core.events.emit(CoreEvent.MessagesChanged(folderId = 1L))
            assertEquals(null, ready(vm).snooze)
        }

    /** #431: when the wake takes the shown row with it, the screen closes rather than offer Snooze. */
    @Test
    fun aWokenMessageWhoseRowIsGoneCloses() =
        runTest {
            val core = FakeMessageCore()
            core.snoozeInfo = SnoozeInfo(wakeAt = 1_800_000_000, deviceOnly = false, waking = true)
            val vm = messageViewModel(core)
            assertEquals(true, ready(vm).snooze?.waking)
            core.snoozeInfoFailure = SnoozeFailure.NotFound()
            core.events.emit(CoreEvent.MessagesChanged(folderId = 1L))
            assertEquals(MessageEffect.Close, vm.effects.first())
        }

    @Test
    fun archiveWithoutAnArchiveFolderExplainsAndStaysOpen() =
        runTest {
            val core = FakeMessageCore().apply { actionFailure = ActionFailure.NoArchiveFolder() }
            val vm = messageViewModel(core)
            vm.archive()
            assertEquals(MessageEffect.ActionRefused(ActionRefusal.NO_ARCHIVE_FOLDER), vm.effects.first())
            assertTrue(vm.state.value is MessageState.Ready)
        }

    @Test
    fun flagShowsAtOnceAndIsUndoneWhenRefused() =
        runTest {
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            vm.toggleFlag()
            assertTrue(ready(vm).message.isFlagged)
            vm.toggleFlag()
            assertFalse(ready(vm).message.isFlagged)
            assertEquals(listOf(MessageAction.Flag, MessageAction.Unflag), core.actions)

            core.actionFailure = ActionFailure.NotFound()
            vm.toggleFlag()
            assertFalse(ready(vm).message.isFlagged)
            assertEquals(MessageEffect.ActionRefused(ActionRefusal.FAILED), vm.effects.first())
        }

    // M4-15: "Mark as read".

    @Test
    fun whenOpenedIsTheDefaultAndShowsTheMessageAsRead() =
        runTest {
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            assertTrue("markRead" in core.calls)
            assertTrue(ready(vm).message.isSeen)
        }

    @Test
    fun afterTwoSecondsLeavesAMessageClosedSoonerUnread() =
        runTest {
            val core = FakeMessageCore()
            val settings = FakeAppSettings().apply { markRead.value = MarkReadMode.AFTER_DELAY }
            val vm = messageViewModel(core, settings)
            assertFalse("not on open", "markRead" in core.calls)

            // The screen runs the delay while the message is shown; leaving cancels it.
            val shortVisit = launch { vm.markReadAfterDelay() }
            advanceTimeBy(1_999)
            runCurrent()
            shortVisit.cancel()
            assertFalse("left after 1.999 s", "markRead" in core.calls)
            assertFalse(ready(vm).message.isSeen)

            launch { vm.markReadAfterDelay() }
            advanceTimeBy(2_001)
            runCurrent()
            assertEquals(1, core.calls.count { it == "markRead" })
            assertTrue(ready(vm).message.isSeen)
        }

    @Test
    fun aMessageShownAgainIsReadAgainAndMarkedReadAgain() =
        runTest {
            // #372 QA: read once, swiped back to unread in the list, opened again. The kept view model still
            // said "read", so the toolbar offered "Mark as unread" at once and the delay never marked it.
            val core = FakeMessageCore()
            val settings = FakeAppSettings().apply { markRead.value = MarkReadMode.AFTER_DELAY }
            val vm = messageViewModel(core, settings)
            vm.onShown()
            launch { vm.markReadAfterDelay() }
            advanceTimeBy(2_001)
            runCurrent()
            assertTrue(ready(vm).message.isSeen)

            core.seen = false
            vm.onShown()
            runCurrent()
            assertFalse("read again from the core", ready(vm).message.isSeen)
            launch { vm.markReadAfterDelay() }
            advanceTimeBy(2_001)
            runCurrent()
            assertEquals(2, core.calls.count { it == "markRead" })
            assertTrue(ready(vm).message.isSeen)
        }

    @Test
    fun onlyByHandNeverMarksOnOpenButTheButtonDoes() =
        runTest {
            val core = FakeMessageCore()
            val settings = FakeAppSettings().apply { markRead.value = MarkReadMode.MANUAL }
            val vm = messageViewModel(core, settings)
            assertFalse("markRead" in core.calls)
            assertFalse(ready(vm).message.isSeen)

            vm.markReadByHand()
            assertTrue("markRead" in core.calls)
            assertTrue(ready(vm).message.isSeen)
        }

    @Test
    fun aFailedMarkReadKeepsTheMessageUnread() =
        runTest {
            val core = FakeMessageCore().apply { markReadFailure = SyncErrorKind.NETWORK }
            val settings = FakeAppSettings().apply { markRead.value = MarkReadMode.MANUAL }
            val vm = messageViewModel(core, settings)
            vm.markReadByHand()
            assertFalse(ready(vm).message.isSeen)
        }

    @Test
    fun aRefusedMoveStaysOnTheMessage() =
        runTest {
            val core = FakeMessageCore().apply { actionFailure = ActionFailure.NotFound() }
            val vm = messageViewModel(core)
            vm.archive()
            assertEquals(MessageEffect.ActionRefused(ActionRefusal.FAILED), vm.effects.first())
            assertTrue(vm.state.value is MessageState.Ready)
        }
}
