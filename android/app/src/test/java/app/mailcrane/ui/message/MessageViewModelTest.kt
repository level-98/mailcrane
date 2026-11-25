package app.mailcrane.ui.message

import app.mailcrane.core.ActionFailure
import app.mailcrane.core.InlinePart
import app.mailcrane.core.LinkCleaner
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageAddress
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageCore
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.SnoozeFailure
import app.mailcrane.core.SnoozeInfo
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.UnsubscribeFailure
import app.mailcrane.core.UnsubscribeInfo
import app.mailcrane.core.UnsubscribeMailto
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.ui.compose.ComposePrefill
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
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
) = MessageViewModel(core, settings, cleaner, ID)

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
)

private class FakeMessageCore(
    var downloaded: Boolean = true,
    var seen: Boolean = false,
    var remoteBlocked: UInt = 2u,
    var plain: Boolean = false,
) : MessageCore {
    val calls = mutableListOf<String>()
    var fetchFailure: SyncErrorKind? = null
    var markReadFailure: SyncErrorKind? = null
    var parts = mapOf<String, InlinePart>()

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
        return detail(body, seen)
    }

    override suspend fun fetchBody(messageId: Long) {
        calls += "fetch"
        fetchFailure?.let { throw SyncFailure.Failed(it) }
        downloaded = true
    }

    override suspend fun markRead(messageId: Long) {
        calls += "markRead"
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

    override suspend fun snoozeInfo(messageId: Long): SnoozeInfo? = snoozeInfo

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
    fun archiveDeleteMoveAndUnreadCloseTheMessage() =
        runTest {
            val core = FakeMessageCore()
            val vm = messageViewModel(core)
            vm.archive()
            assertEquals(MessageEffect.Close, vm.effects.first())
            vm.delete()
            assertEquals(MessageEffect.Close, vm.effects.first())
            vm.moveTo(9)
            assertEquals(MessageEffect.Close, vm.effects.first())
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
}
