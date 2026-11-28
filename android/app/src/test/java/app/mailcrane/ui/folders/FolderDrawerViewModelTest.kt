package app.mailcrane.ui.folders

import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
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

private const val ACCOUNT = 3L

private fun folder(
    id: Long,
    name: String,
    role: FolderRole? = null,
    unread: Int = 0,
) = FolderSummary(id = id, displayName = name, role = role, depth = 0u, unreadCount = unread.toUInt())

private val INBOX = folder(1, "INBOX", FolderRole.INBOX, unread = 3)
private val SENT = folder(2, "Sent Items", FolderRole.SENT)
private val WORK = folder(5, "Work")

/** An account whose stored folders are [stored]; `refreshFolders` replaces them with [onServer]. */
private class FakeFolders(
    var stored: List<FolderSummary> = emptyList(),
    var onServer: List<FolderSummary> = stored,
) : MailboxCore {
    override val events = MutableSharedFlow<CoreEvent>(extraBufferCapacity = 16)
    var refreshGate: CompletableDeferred<Unit>? = null
    var refreshFailure: SyncErrorKind? = null
    var listFailure: SyncErrorKind? = null
    var messagesFailure: SyncErrorKind? = null
    var refreshCalls = 0

    override suspend fun listFolders(accountId: Long): List<FolderSummary> {
        assertEquals(ACCOUNT, accountId)
        listFailure?.let { throw SyncFailure.Failed(it) }
        return stored
    }

    override suspend fun swipeAction(
        messageId: Long,
        action: app.mailcrane.core.MessageAction,
        undoable: Boolean,
    ) = Unit

    override suspend fun searchMessages(
        accountId: Long,
        folderId: Long?,
        query: String,
        limit: Int,
    ) = emptyList<app.mailcrane.core.MessageSummary>()

    override suspend fun searchServer(
        folderId: Long,
        query: String,
    ) = 0

    override suspend fun undoAction(messageId: Long) = false

    override suspend fun refreshFolders(accountId: Long): Long? {
        refreshCalls++
        refreshGate?.await()
        refreshFailure?.let { throw SyncFailure.Failed(it) }
        stored = onServer
        return onServer.firstOrNull { it.role == FolderRole.INBOX }?.id
    }

    override suspend fun listMessages(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<MessageSummary> {
        messagesFailure?.let { throw SyncFailure.Failed(it) }
        return emptyList()
    }

    override suspend fun syncFolder(folderId: Long) = Unit

    override suspend fun loadOlder(
        folderId: Long,
        count: Int,
    ): Int = 0
}

@OptIn(ExperimentalCoroutinesApi::class)
class FolderDrawerViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun showsStoredFoldersAtOnceWithTheInboxOpenWhileRefreshing() {
        val core = FakeFolders(stored = listOf(INBOX, SENT, WORK))
        core.refreshGate = CompletableDeferred()
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        val state = vm.state.value
        assertFalse(state.isLoading)
        assertEquals(listOf(INBOX, SENT, WORK), state.folders)
        assertEquals(INBOX, state.selected)
        assertEquals(1, core.refreshCalls)
    }

    @Test
    fun firstRefreshLoadsTheFoldersAndOpensTheInbox() {
        val core = FakeFolders(onServer = listOf(INBOX, WORK))
        val gate = CompletableDeferred<Unit>()
        core.refreshGate = gate
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        assertTrue(vm.state.value.isLoading)
        assertNull(vm.state.value.selected)
        gate.complete(Unit)
        assertFalse(vm.state.value.isLoading)
        assertEquals(listOf(INBOX, WORK), vm.state.value.folders)
        assertEquals(INBOX.id, vm.state.value.selectedFolderId)
    }

    @Test
    fun choosingAFolderOpensIt() {
        val vm = FolderDrawerViewModel(FakeFolders(stored = listOf(INBOX, SENT, WORK)), ACCOUNT)
        vm.select(WORK.id)
        assertEquals(WORK, vm.state.value.selected)
        vm.select(404L) // Not one of the account's folders, so ignored.
        assertEquals(WORK, vm.state.value.selected)
    }

    @Test
    fun unreadCountsFollowCoreEvents() {
        val core = FakeFolders(stored = listOf(INBOX, WORK))
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        vm.select(WORK.id)
        core.stored = listOf(INBOX.copy(unreadCount = 2u), WORK)
        core.events.tryEmit(CoreEvent.MessagesChanged(INBOX.id))
        assertEquals(
            2u,
            vm.state.value.folders
                .first()
                .unreadCount,
        )
        core.stored = listOf(INBOX.copy(unreadCount = 0u), WORK)
        core.events.tryEmit(CoreEvent.FolderChanged(INBOX.id))
        assertEquals(
            0u,
            vm.state.value.folders
                .first()
                .unreadCount,
        )
        // The open folder stays open.
        assertEquals(WORK.id, vm.state.value.selectedFolderId)
    }

    @Test
    fun openFolderThatDisappearsFallsBackToTheInbox() {
        val core = FakeFolders(stored = listOf(INBOX, WORK))
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        vm.select(WORK.id)
        core.stored = listOf(INBOX)
        core.events.tryEmit(CoreEvent.FolderChanged(WORK.id))
        assertEquals(INBOX.id, vm.state.value.selectedFolderId)
    }

    @Test
    fun accountWithoutAnInboxOpensItsFirstFolder() {
        val vm = FolderDrawerViewModel(FakeFolders(onServer = listOf(SENT, WORK)), ACCOUNT)
        assertEquals(SENT.id, vm.state.value.selectedFolderId)
    }

    @Test
    fun failedRefreshKeepsStoredFolders() {
        val core = FakeFolders(stored = listOf(INBOX, WORK))
        core.refreshFailure = SyncErrorKind.NETWORK
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        assertNull(vm.state.value.loadError)
        assertEquals(INBOX, vm.state.value.selected)
    }

    @Test
    fun failedFirstRefreshShowsTheErrorAndRetryRecovers() {
        val core = FakeFolders(onServer = listOf(INBOX))
        core.refreshFailure = SyncErrorKind.AUTH
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        assertFalse(vm.state.value.isLoading)
        assertEquals(SyncErrorKind.AUTH, vm.state.value.loadError)
        assertNull(vm.state.value.selected)

        core.refreshFailure = null
        vm.retry()
        assertNull(vm.state.value.loadError)
        assertEquals(INBOX, vm.state.value.selected)
    }

    @Test
    fun unreadableDatabaseWithNoFoldersShowsTheError() {
        val core = FakeFolders(onServer = listOf(INBOX))
        core.listFailure = SyncErrorKind.STORAGE
        val vm = FolderDrawerViewModel(core, ACCOUNT)
        assertFalse(vm.state.value.isLoading)
        assertEquals(SyncErrorKind.STORAGE, vm.state.value.loadError)
    }

    @Test
    fun accountWithNoFoldersIsNotLoadingAndHasNoError() {
        val vm = FolderDrawerViewModel(FakeFolders(), ACCOUNT)
        assertFalse(vm.state.value.isLoading)
        assertNull(vm.state.value.loadError)
        assertNull(vm.state.value.selected)
    }

    @Test
    fun aListThatCantBeReadAfterAMoveGivesNoNeighbour() =
        runTest {
            // #566 review: a failed read must not crash the app; empty means the list shows.
            val core = FakeFolders(stored = listOf(INBOX))
            val vm = FolderDrawerViewModel(core, ACCOUNT)
            core.messagesFailure = SyncErrorKind.STORAGE
            assertEquals(emptyList<Long>(), vm.idsNow(Origin(INBOX.id, null, listOf(10L, 20L))))
        }
}
