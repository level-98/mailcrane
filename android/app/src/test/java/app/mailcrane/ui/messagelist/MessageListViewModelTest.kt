package app.mailcrane.ui.messagelist

import app.mailcrane.core.CoreEvent
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
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

private const val FOLDER = 7L

private fun summary(id: Long) =
    MessageSummary(
        id = id,
        fromName = "Ana",
        fromAddr = "ana@example.org",
        subject = "Message $id",
        snippet = "Hello",
        date = id,
        isSeen = false,
        isFlagged = false,
        hasAttachments = false,
    )

/** A folder of [stored] messages (newest = highest id) and [onServer] more that `loadOlder` can fetch. */
private class FakeMailbox(
    stored: Int,
    var onServer: Int = 0,
) : MailboxCore {
    var messages: List<MessageSummary> = (stored downTo 1).map { summary(it.toLong()) }
    override val events = MutableSharedFlow<CoreEvent>(extraBufferCapacity = 16)
    var syncCalls = 0
    var loadOlderCalls = 0
    var syncGate: CompletableDeferred<Unit>? = null
    var loadOlderGate: CompletableDeferred<Unit>? = null
    var syncFailure: SyncErrorKind? = null
    var listFailure: SyncErrorKind? = null
    var rowsRead = 0

    override suspend fun listMessages(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<MessageSummary> {
        assertEquals(FOLDER, folderId)
        assertTrue("the core caps a page at 500", limit <= 500)
        listFailure?.let { throw SyncFailure.Failed(it) }
        return messages.drop(offset).take(limit).also { rowsRead += it.size }
    }

    override suspend fun syncFolder(folderId: Long) {
        syncCalls++
        syncGate?.await()
        syncFailure?.let { throw SyncFailure.Failed(it) }
    }

    override suspend fun loadOlder(
        folderId: Long,
        count: Int,
    ): Int {
        loadOlderCalls++
        loadOlderGate?.await()
        val n = minOf(count, onServer)
        val oldest = messages.minOfOrNull { it.id } ?: 1L
        messages = messages + (1..n).map { summary(oldest - it) }
        onServer -= n
        return n
    }

    override suspend fun refreshFolders(accountId: Long): Long? = FOLDER
}

@OptIn(ExperimentalCoroutinesApi::class)
class MessageListViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun showsFirstPageNewestFirstAndSyncsOnOpen() {
        val core = FakeMailbox(stored = 120)
        val vm = MessageListViewModel(core, FOLDER)
        val state = vm.state.value
        assertFalse(state.isLoading)
        assertFalse(state.isRefreshing)
        assertEquals(MessageListViewModel.PAGE_SIZE, state.messages.size)
        assertEquals(120L, state.messages.first().id)
        assertEquals(1, core.syncCalls)
    }

    @Test
    fun refreshShowsProgressUntilSyncFinishes() {
        val core = FakeMailbox(stored = 3)
        val vm = MessageListViewModel(core, FOLDER)
        val gate = CompletableDeferred<Unit>()
        core.syncGate = gate
        vm.refresh()
        assertTrue(vm.state.value.isRefreshing)
        vm.refresh() // ignored while one is running
        assertEquals(2, core.syncCalls)
        gate.complete(Unit)
        assertFalse(vm.state.value.isRefreshing)
    }

    @Test
    fun failedRefreshStopsProgressAndReportsTheError() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FOLDER)
            core.syncFailure = SyncErrorKind.NETWORK
            vm.refresh()
            assertFalse(vm.state.value.isRefreshing)
            assertEquals(SyncErrorKind.NETWORK, vm.errors.first())
            assertEquals(3, vm.state.value.messages.size)
        }

    @Test
    fun scrollingShowsMoreStoredMessagesBeforeAskingTheServer() {
        val core = FakeMailbox(stored = 120, onServer = 100)
        val vm = MessageListViewModel(core, FOLDER)
        vm.loadMore()
        assertEquals(100, vm.state.value.messages.size)
        assertEquals(0, core.loadOlderCalls)
        vm.loadMore()
        assertEquals(120, vm.state.value.messages.size)
        assertEquals(0, core.loadOlderCalls)
        vm.loadMore()
        assertEquals(1, core.loadOlderCalls)
        assertEquals(170, vm.state.value.messages.size)
    }

    @Test
    fun loadOlderIsCalledOncePerPageWhileOneIsRunning() {
        val core = FakeMailbox(stored = 10, onServer = 100)
        val vm = MessageListViewModel(core, FOLDER)
        val gate = CompletableDeferred<Unit>()
        core.loadOlderGate = gate
        vm.loadMore()
        vm.loadMore()
        vm.loadMore()
        assertTrue(vm.state.value.isLoadingOlder)
        assertEquals(1, core.loadOlderCalls)
        gate.complete(Unit)
        assertFalse(vm.state.value.isLoadingOlder)
        assertEquals(60, vm.state.value.messages.size)
    }

    @Test
    fun stopsAskingOnceTheServerHasNoOlderMessages() {
        val core = FakeMailbox(stored = 10, onServer = 0)
        val vm = MessageListViewModel(core, FOLDER)
        vm.loadMore()
        assertTrue(vm.state.value.endReached)
        vm.loadMore()
        assertEquals(1, core.loadOlderCalls)
    }

    @Test
    fun messagesChangedReloadsTheLoadedRange() {
        val core = FakeMailbox(stored = 120)
        val vm = MessageListViewModel(core, FOLDER)
        vm.loadMore()
        assertEquals(100, vm.state.value.messages.size)

        core.messages = listOf(summary(500)) + core.messages.map { if (it.id == 119L) it.copy(isSeen = true) else it }
        core.events.tryEmit(CoreEvent.MessagesChanged(FOLDER))

        val messages = vm.state.value.messages
        assertEquals(100, messages.size) // the loaded range is kept, not reset to one page
        assertEquals(500L, messages.first().id)
        assertTrue(messages.first { it.id == 119L }.isSeen)
    }

    @Test
    fun eventsForOtherFoldersAreIgnored() {
        val core = FakeMailbox(stored = 3)
        val vm = MessageListViewModel(core, FOLDER)
        core.messages = emptyList()
        core.events.tryEmit(CoreEvent.MessagesChanged(FOLDER + 1))
        core.events.tryEmit(CoreEvent.SyncError(1, SyncErrorKind.AUTH))
        assertEquals(3, vm.state.value.messages.size)
    }

    @Test
    fun readsALargeFolderInChunksTheCoreAccepts() {
        val core = FakeMailbox(stored = 5_000)
        val vm = MessageListViewModel(core, FOLDER)
        repeat(30) { vm.loadMore() }
        assertEquals(31 * MessageListViewModel.PAGE_SIZE, vm.state.value.messages.size)
        assertEquals(
            (5_000L downTo 3_451L).toList(),
            vm.state.value.messages
                .map { it.id },
        )
    }

    @Test
    fun scrollingReadsOnlyTheNextPage() {
        val core = FakeMailbox(stored = 5_000)
        val vm = MessageListViewModel(core, FOLDER)
        val before = core.rowsRead
        repeat(30) { vm.loadMore() }
        assertEquals(30 * MessageListViewModel.PAGE_SIZE, core.rowsRead - before)
    }

    @Test
    fun failedFirstReadShowsAnErrorState() {
        val core = FakeMailbox(stored = 3)
        core.listFailure = SyncErrorKind.STORAGE
        val vm = MessageListViewModel(core, FOLDER)
        assertEquals(SyncErrorKind.STORAGE, vm.state.value.loadError)
        assertFalse(vm.state.value.isLoading)

        core.listFailure = null
        vm.refresh()
        assertNull(vm.state.value.loadError)
        assertEquals(3, vm.state.value.messages.size)
    }
}
