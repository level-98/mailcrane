package app.mailcrane.ui.message

import app.mailcrane.core.InlinePart
import app.mailcrane.core.MessageAddress
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageCore
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
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

private fun detail(
    body: MessageBody,
    isSeen: Boolean = false,
) = MessageDetail(
    id = ID,
    folderId = 7,
    from = MessageAddress("Ana", "ana@example.org"),
    to = listOf(MessageAddress("", "bob@example.com")),
    cc = emptyList(),
    date = 1_772_445_600,
    subject = "Hi",
    isSeen = isSeen,
    body = body,
    attachments = emptyList(),
)

private fun html(
    remoteBlocked: UInt,
    allowRemote: Boolean,
) = MessageBody.Html(html = "<p>remote=$allowRemote</p>", remoteBlocked = remoteBlocked, trackersRemoved = 0u)

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
            val vm = MessageViewModel(core, ID)
            val state = ready(vm)
            assertFalse(state.remoteAllowed)
            assertTrue(state.showRemoteBanner)
            assertEquals(listOf("get(remote=false)", "markRead"), core.calls)
        }

    @Test
    fun downloads_the_body_first_when_it_is_not_stored() =
        runTest {
            val core = FakeMessageCore(downloaded = false)
            val vm = MessageViewModel(core, ID)
            assertTrue(ready(vm).message.body is MessageBody.Html)
            assertEquals(listOf("get(remote=false)", "fetch", "get(remote=false)", "markRead"), core.calls)
        }

    @Test
    fun a_read_message_is_not_marked_again() =
        runTest {
            val core = FakeMessageCore(seen = true)
            MessageViewModel(core, ID)
            assertFalse("markRead" in core.calls)
        }

    @Test
    fun load_remote_images_refetches_with_remote_allowed_for_this_message() =
        runTest {
            val core = FakeMessageCore()
            val vm = MessageViewModel(core, ID)
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
            val other = MessageViewModel(FakeMessageCore(), ID)
            assertFalse(ready(other).remoteAllowed)
        }

    @Test
    fun no_banner_when_nothing_was_blocked_or_for_plain_text() =
        runTest {
            assertFalse(ready(MessageViewModel(FakeMessageCore(remoteBlocked = 0u), ID)).showRemoteBanner)
            assertFalse(ready(MessageViewModel(FakeMessageCore(plain = true), ID)).showRemoteBanner)
        }

    @Test
    fun a_failed_download_shows_the_error_and_retry_recovers() =
        runTest {
            val core = FakeMessageCore(downloaded = false)
            core.fetchFailure = SyncErrorKind.NETWORK
            val vm = MessageViewModel(core, ID)
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
            val vm = MessageViewModel(core, ID)
            assertTrue(vm.state.value is MessageState.Ready)
        }

    @Test
    fun inline_parts_come_from_this_message() =
        runTest {
            val png = InlinePart("image/png", byteArrayOf(1, 2, 3))
            val core = FakeMessageCore().apply { parts = mapOf("logo@example.org" to png) }
            val vm = MessageViewModel(core, ID)
            assertEquals(png, vm.inlinePart("logo@example.org"))
            assertNull(vm.inlinePart("other@example.org"))
        }
}
