package app.mailcrane.ui.messagelist

import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.ActionKind
import app.mailcrane.core.BulkApplied
import app.mailcrane.core.BulkSelection
import app.mailcrane.core.BulkUndo
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.ThreadSummary
import app.mailcrane.core.UNIFIED_FOLDER_ID
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.SwipeAction
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
    var threads: List<ThreadSummary> = emptyList()
    var unified: List<MessageSummary> = emptyList()
    var unifiedSyncs = 0

    override suspend fun listUnifiedInbox(
        offset: Int,
        limit: Int,
    ): List<MessageSummary> = unified.drop(offset).take(limit)

    override suspend fun syncUnifiedInbox() {
        unifiedSyncs++
    }

    var threadReads = 0

    override suspend fun listThreads(
        folderId: Long,
        offset: Int,
        limit: Int,
    ): List<ThreadSummary> {
        threadReads++
        return threads.drop(offset).take(limit)
    }

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

    override suspend fun listFolders(accountId: Long): List<FolderSummary> = accountFolders

    val swiped = mutableListOf<Pair<MessageAction, Boolean>>()
    var actionFailure: ActionFailure? = null
    var undoResult = true

    override suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    ) {
        actionFailure?.let { throw it }
        swiped += action to undoable
    }

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

    override suspend fun undoAction(messageId: Long) = undoResult

    val bulkCalls = mutableListOf<Triple<BulkSelection, MessageAction, Boolean>>()
    var deletedForGood = false

    override suspend fun bulkAction(
        selection: BulkSelection,
        action: MessageAction,
        undoable: Boolean,
    ): BulkApplied {
        actionFailure?.let { throw it }
        bulkCalls += Triple(selection, action, undoable)
        // A conversation row stands for its messages: here, thread n holds message n * 10.
        return BulkApplied(listOf(1L), selection.messageIds + selection.threadIds.map { it * 10 }, deletedForGood)
    }

    val undoBulkCalls = mutableListOf<List<Long>>()
    var undoBulkResult = BulkUndo(0u, 0u)

    override suspend fun undoBulk(messageIds: List<Long>): BulkUndo {
        undoBulkCalls += messageIds
        return undoBulkResult
    }

    var accountFolders: List<FolderSummary> = emptyList()
    var forGood = 0

    override suspend fun countDeletedForGood(selection: BulkSelection): Int {
        actionFailure?.let { throw it }
        return forGood
    }
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
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
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
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
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
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            core.syncFailure = SyncErrorKind.NETWORK
            vm.refresh()
            assertFalse(vm.state.value.isRefreshing)
            assertEquals(SyncErrorKind.NETWORK, vm.errors.first())
            assertEquals(3, vm.state.value.messages.size)
        }

    @Test
    fun aBusyDatabaseIsNotReportedAsAnError() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            // Another sync was saving at the same time (issue #282): no snackbar.
            core.syncFailure = SyncErrorKind.BUSY
            vm.refresh()
            assertFalse(vm.state.value.isRefreshing)
            // The next real failure is the first error anyone sees.
            core.syncFailure = SyncErrorKind.NETWORK
            vm.refresh()
            assertEquals(SyncErrorKind.NETWORK, vm.errors.first())
        }

    @Test
    fun scrollingShowsMoreStoredMessagesBeforeAskingTheServer() {
        val core = FakeMailbox(stored = 120, onServer = 100)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
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
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
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
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.loadMore()
        assertTrue(vm.state.value.endReached)
        vm.loadMore()
        assertEquals(1, core.loadOlderCalls)
    }

    @Test
    fun messagesChangedReloadsTheLoadedRange() {
        val core = FakeMailbox(stored = 120)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
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
    fun anUndoneActionIsReportedOnce() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            core.events.tryEmit(CoreEvent.ActionFailed(1, ActionKind.MOVE, listOf(7L), SyncErrorKind.PROTOCOL))
            vm.actionsUndone.first()
            assertEquals(3, vm.state.value.messages.size)
        }

    @Test
    fun eventsForOtherFoldersAreIgnored() {
        val core = FakeMailbox(stored = 3)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        core.messages = emptyList()
        core.events.tryEmit(CoreEvent.MessagesChanged(FOLDER + 1))
        core.events.tryEmit(CoreEvent.SyncError(1, SyncErrorKind.AUTH))
        assertEquals(3, vm.state.value.messages.size)
    }

    @Test
    fun readsALargeFolderInChunksTheCoreAccepts() {
        val core = FakeMailbox(stored = 5_000)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
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
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        val before = core.rowsRead
        repeat(30) { vm.loadMore() }
        assertEquals(30 * MessageListViewModel.PAGE_SIZE, core.rowsRead - before)
    }

    @Test
    fun failedFirstReadShowsAnErrorState() {
        val core = FakeMailbox(stored = 3)
        core.listFailure = SyncErrorKind.STORAGE
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        assertEquals(SyncErrorKind.STORAGE, vm.state.value.loadError)
        assertFalse(vm.state.value.isLoading)

        core.listFailure = null
        vm.refresh()
        assertNull(vm.state.value.loadError)
        assertEquals(3, vm.state.value.messages.size)
    }

    @Test
    fun swipesRunThroughTheCoreAndArchiveOffersUndo() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            val unread = summary(1)
            vm.swipe(unread, SwipeAction.READ_UNREAD)
            vm.swipe(unread.copy(isSeen = true), SwipeAction.READ_UNREAD)
            vm.swipe(unread, SwipeAction.FLAG)
            vm.swipe(unread.copy(isFlagged = true), SwipeAction.FLAG)
            vm.swipe(unread, SwipeAction.NONE)
            vm.move(1, 9)
            assertEquals(
                listOf(
                    MessageAction.MarkRead to false,
                    MessageAction.MarkUnread to false,
                    MessageAction.Flag to false,
                    MessageAction.Unflag to false,
                    MessageAction.Move(9) to false,
                ),
                core.swiped,
            )
            vm.swipe(unread, SwipeAction.ARCHIVE)
            assertEquals(UndoOffer(1, SwipeAction.ARCHIVE), vm.undoOffers.first())
            vm.swipe(unread, SwipeAction.DELETE)
            assertEquals(UndoOffer(1, SwipeAction.DELETE), vm.undoOffers.first())
            assertEquals(listOf(MessageAction.Archive to true, MessageAction.Delete to true), core.swiped.takeLast(2))
        }

    @Test
    fun undoTooLateAndRefusedSwipesAreExplained() =
        runTest {
            val core = FakeMailbox(stored = 3).apply { undoResult = false }
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.undo(1)
            assertEquals(app.mailcrane.R.string.swipe_undo_too_late, vm.notices.first())
            core.actionFailure = ActionFailure.NoArchiveFolder()
            vm.swipe(summary(1), SwipeAction.ARCHIVE)
            assertEquals(app.mailcrane.R.string.action_no_archive_folder, vm.notices.first())
        }

    @Test
    fun swipeSettingsReachTheList() {
        val settings = FakeAppSettings()
        val vm = MessageListViewModel(FakeMailbox(stored = 1), settings, FOLDER)
        assertEquals(SwipeAction.READ_UNREAD to SwipeAction.ARCHIVE, vm.swipes.value)
        kotlinx.coroutines.runBlocking { settings.setSwipeLeft(SwipeAction.DELETE) }
        assertEquals(SwipeAction.READ_UNREAD to SwipeAction.DELETE, vm.swipes.value)
    }

    private fun thread(
        id: Long,
        count: UInt,
    ) = ThreadSummary(id, "Subject $id", listOf("Ana", "Bob"), count, 1u, id, false, false, id * 10)

    @Test
    fun conversationsFollowTheSetting() =
        runTest {
            val core = FakeMailbox(stored = 3).apply { threads = listOf(thread(2, 2u), thread(1, 1u)) }
            val settings = FakeAppSettings()
            val vm = MessageListViewModel(core, settings, FOLDER)
            assertTrue(vm.state.value.grouped)
            assertEquals(
                listOf(2L, 1L),
                vm.state.value.threads
                    .map { it.threadId },
            )
            settings.setGroupByConversation(false)
            assertFalse(vm.state.value.grouped)
            assertTrue(
                vm.state.value.threads
                    .isEmpty(),
            )
            assertEquals(3, vm.state.value.messages.size)
        }

    @Test
    fun theUnifiedInboxListsEveryAccountAndSyncsThemAll() =
        runTest {
            val core =
                FakeMailbox(stored = 0).apply {
                    unified =
                        listOf(
                            summary(9).copy(accountId = 1, accountColor = 1u),
                            summary(8).copy(accountId = 2, accountColor = 2u),
                        )
                    threads = listOf(thread(1, 2u))
                }
            val vm = MessageListViewModel(core, FakeAppSettings(), UNIFIED_FOLDER_ID)
            assertEquals(
                listOf(9L, 8L),
                vm.state.value.messages
                    .map { it.id },
            )
            assertTrue(
                "no conversations in the unified inbox",
                vm.state.value.threads
                    .isEmpty(),
            )
            vm.refresh()
            assertEquals("once on opening, once on refresh", 2, core.unifiedSyncs)
            assertEquals(0, core.syncCalls)
        }
    // Multi-select (M4-14).

    private fun folder(
        id: Long,
        role: FolderRole? = null,
    ) = FolderSummary(id = id, displayName = "F$id", role = role, depth = 0u, unreadCount = 0u)

    @Test
    fun longPressStartsSelectionTapsToggleAndClearEndsIt() {
        val vm = MessageListViewModel(FakeMailbox(stored = 5), FakeAppSettings(), FOLDER)
        assertFalse(vm.state.value.selecting)
        vm.toggleMessage(5)
        vm.toggleMessage(4)
        assertTrue(vm.state.value.selecting)
        assertEquals(2, vm.state.value.selectedCount)
        vm.toggleMessage(5)
        assertEquals(setOf(4L), vm.state.value.selectedMessages)
        vm.clearSelection()
        assertFalse(vm.state.value.selecting)
    }

    @Test
    fun aBulkActionIsOneCoreCallForEverySelectedRowAndEndsSelection() {
        val core = FakeMailbox(stored = 5)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.toggleMessage(5)
        vm.toggleMessage(3)
        vm.toggleThread(42)
        vm.bulk(BulkAction.READ)
        assertEquals(1, core.bulkCalls.size)
        val (selection, action, undoable) = core.bulkCalls.single()
        assertEquals(setOf(5L, 3L), selection.messageIds.toSet())
        assertEquals(listOf(42L), selection.threadIds)
        assertEquals(FOLDER, selection.folderId)
        assertEquals(MessageAction.MarkRead, action)
        assertFalse("read needs no undo", undoable)
        assertFalse(vm.state.value.selecting)
    }

    @Test
    fun readAndFlagTurnIntoUnreadAndUnflagWhenEverySelectedRowIs() {
        val core = FakeMailbox(stored = 3)
        core.messages = core.messages.map { it.copy(isSeen = true, isFlagged = it.id != 1L) }
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.toggleMessage(3)
        vm.toggleMessage(2)
        assertTrue(vm.state.value.allSelectedRead)
        assertTrue(vm.state.value.allSelectedFlagged)
        vm.toggleMessage(1)
        assertFalse(vm.state.value.allSelectedFlagged)
    }

    @Test
    fun archiveOffersUndoForEveryMessageAndUndoTakesThemBack() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.toggleMessage(3)
            vm.toggleMessage(2)
            vm.bulk(BulkAction.ARCHIVE)
            assertEquals(MessageAction.Archive, core.bulkCalls.single().second)
            assertTrue("waits for a possible undo", core.bulkCalls.single().third)
            val offer = vm.bulkUndoOffers.first()
            assertEquals(BulkUndoKind.ARCHIVED, offer.kind)
            assertEquals(setOf(3L, 2L), offer.messageIds.toSet())
            core.undoBulkResult = BulkUndo(2u, 0u)
            vm.undoBulk(offer.messageIds)
            assertEquals(listOf(offer.messageIds), core.undoBulkCalls)
        }

    @Test
    fun undoThatCameTooLateForSomeSaysSo() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            core.undoBulkResult = BulkUndo(1u, 1u)
            vm.undoBulk(listOf(3, 2))
            assertEquals(R.string.bulk_undo_partly_late, vm.notices.first())
            core.undoBulkResult = BulkUndo(0u, 2u)
            vm.undoBulk(listOf(3, 2))
            assertEquals(R.string.swipe_undo_too_late, vm.notices.first())
        }

    @Test
    fun deleteThatDeletedForGoodSaysSo() =
        runTest {
            val core = FakeMailbox(stored = 3).apply { deletedForGood = true }
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.toggleMessage(3)
            vm.bulk(BulkAction.DELETE)
            assertEquals(BulkUndoKind.DELETED_FOR_GOOD, vm.bulkUndoOffers.first().kind)
        }

    @Test
    fun aRefusedBulkActionSaysWhy() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            core.actionFailure = ActionFailure.NoArchiveFolder()
            vm.toggleMessage(3)
            vm.bulk(BulkAction.ARCHIVE)
            assertEquals(R.string.action_no_archive_folder, vm.notices.first())
            core.actionFailure = ActionFailure.MixedAccounts()
            vm.toggleMessage(3)
            vm.bulkMove(9)
            assertEquals(R.string.bulk_move_one_account, vm.notices.first())
        }

    @Test
    fun rowsThatLeaveTheListLeaveTheSelection() {
        val core = FakeMailbox(stored = 3)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.toggleMessage(3)
        vm.toggleMessage(2)
        core.messages = core.messages.filter { it.id != 3L }
        core.events.tryEmit(CoreEvent.MessagesChanged(FOLDER))
        assertEquals(setOf(2L), vm.state.value.selectedMessages)
        core.messages = emptyList()
        core.events.tryEmit(CoreEvent.MessagesChanged(FOLDER))
        assertFalse("nothing left: selection ends", vm.state.value.selecting)
    }

    @Test
    fun moveInAFolderOffersTheOtherFolders() {
        val core = FakeMailbox(stored = 3)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.toggleMessage(3)
        vm.startMove(listOf(folder(FOLDER, FolderRole.INBOX), folder(8)))
        assertEquals(listOf(8L), vm.moveTargets.value?.map { it.id })
        vm.bulkMove(8)
        assertNull(vm.moveTargets.value)
        assertEquals(MessageAction.Move(8), core.bulkCalls.single().second)
    }

    @Test
    fun moveInTheUnifiedInboxNeedsOneAccount() =
        runTest {
            val core =
                FakeMailbox(stored = 0).apply {
                    unified =
                        listOf(
                            summary(9).copy(accountId = 1, accountColor = 1u),
                            summary(8).copy(accountId = 2, accountColor = 2u),
                            summary(7).copy(accountId = 2, accountColor = 2u),
                        )
                    accountFolders = listOf(folder(20, FolderRole.INBOX), folder(21))
                }
            val vm = MessageListViewModel(core, FakeAppSettings(), UNIFIED_FOLDER_ID)
            vm.toggleMessage(9)
            vm.toggleMessage(8)
            vm.startMove(emptyList())
            assertEquals(R.string.bulk_move_one_account, vm.notices.first())
            assertNull(vm.moveTargets.value)
            vm.toggleMessage(9)
            vm.toggleMessage(7)
            vm.startMove(emptyList())
            assertEquals("that account's folders, not its inbox", listOf(21L), vm.moveTargets.value?.map { it.id })
        }

    @Test
    fun selectionSurvivesWhatARotationKeeps() {
        // Rotation keeps the view model; the selection lives there, not in the composable.
        val core = FakeMailbox(stored = 3)
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.toggleMessage(2)
        vm.refresh()
        assertEquals(setOf(2L), vm.state.value.selectedMessages)
    }

    @Test
    fun aDeleteThatWouldBePermanentAsksFirstAndOffersNoUndo() =
        runTest {
            val core = FakeMailbox(stored = 3).apply { forGood = 2 }
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.toggleMessage(3)
            vm.toggleMessage(2)
            vm.bulk(BulkAction.DELETE)
            assertEquals("asks before anything is deleted", 2, vm.state.value.confirmDeleteForGood)
            assertTrue(core.bulkCalls.isEmpty())
            vm.cancelDeleteForGood()
            assertEquals(0, vm.state.value.confirmDeleteForGood)
            assertEquals("cancel keeps the selection", 2, vm.state.value.selectedCount)

            vm.bulk(BulkAction.DELETE)
            core.deletedForGood = true
            vm.confirmDeleteForGood()
            val (_, action, undoable) = core.bulkCalls.single()
            assertEquals(MessageAction.Delete, action)
            assertFalse("no undo window for a permanent delete", undoable)
            assertEquals(BulkUndoKind.DELETED_FOR_GOOD, vm.bulkUndoOffers.first().kind)
            assertFalse(vm.state.value.selecting)
        }

    @Test
    fun aDeleteToTrashDoesNotAskAndOffersUndo() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.toggleMessage(3)
            vm.bulk(BulkAction.DELETE)
            assertEquals(0, vm.state.value.confirmDeleteForGood)
            assertTrue(core.bulkCalls.single().third)
            assertEquals(BulkUndoKind.DELETED, vm.bulkUndoOffers.first().kind)
        }

    @Test
    fun clearingTheSelectionDropsThePermanentDeleteQuestion() {
        val core = FakeMailbox(stored = 3).apply { forGood = 1 }
        val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
        vm.toggleMessage(3)
        vm.bulk(BulkAction.DELETE)
        assertEquals(1, vm.state.value.confirmDeleteForGood)
        vm.clearSelection()
        assertEquals(0, vm.state.value.confirmDeleteForGood)
    }

    /** #552: a conversation row of thread 4 (message 40), unread or flagged as asked. */
    private fun swipedThread(
        unread: UInt = 0u,
        flagged: Boolean = false,
    ) = ThreadSummary(
        threadId = 4,
        subject = "Subject 4",
        participants = listOf("Ana", "Bob"),
        messageCount = 3u,
        unreadCount = unread,
        lastDate = 4,
        hasAttachments = false,
        isFlagged = flagged,
        newestMessageId = 40,
    )

    @Test
    fun anArchiveSwipeOnAConversationArchivesItsMessagesWithUndo() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.swipeThread(swipedThread(), SwipeAction.ARCHIVE)
            val (selection, action, undoable) = core.bulkCalls.single()
            assertEquals(listOf(4L), selection.threadIds)
            assertEquals(FOLDER, selection.folderId)
            assertTrue(selection.messageIds.isEmpty())
            assertEquals(MessageAction.Archive, action)
            assertTrue("waits for a possible undo", undoable)
            val offer = vm.bulkUndoOffers.first()
            assertEquals(BulkUndoKind.ARCHIVED, offer.kind)
            assertEquals(listOf(40L), offer.messageIds)
            assertFalse("a swipe doesn't start selection", vm.state.value.selecting)
        }

    @Test
    fun readAndFlagSwipesOnAConversationGoByTheRow() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.swipeThread(swipedThread(unread = 2u), SwipeAction.READ_UNREAD)
            vm.swipeThread(swipedThread(unread = 0u), SwipeAction.READ_UNREAD)
            vm.swipeThread(swipedThread(flagged = false), SwipeAction.FLAG)
            vm.swipeThread(swipedThread(flagged = true), SwipeAction.FLAG)
            assertEquals(
                listOf(MessageAction.MarkRead, MessageAction.MarkUnread, MessageAction.Flag, MessageAction.Unflag),
                core.bulkCalls.map { it.second },
            )
            assertTrue("no undo for read and flag", core.bulkCalls.none { it.third })
        }

    @Test
    fun aDeleteSwipeOnAConversationThatWouldBePermanentAsksFirst() =
        runTest {
            val core = FakeMailbox(stored = 3).apply { forGood = 3 }
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.swipeThread(swipedThread(), SwipeAction.DELETE)
            assertEquals(3, vm.state.value.confirmDeleteForGood)
            assertTrue("nothing deleted before the answer", core.bulkCalls.isEmpty())
            vm.cancelDeleteForGood()
            assertTrue(core.bulkCalls.isEmpty())

            vm.swipeThread(swipedThread(), SwipeAction.DELETE)
            vm.confirmDeleteForGood()
            val (selection, action, undoable) = core.bulkCalls.single()
            assertEquals("deletes the swiped conversation", listOf(4L), selection.threadIds)
            assertEquals(MessageAction.Delete, action)
            assertFalse(undoable)
        }

    @Test
    fun aDeleteSwipeOnAConversationToTrashOffersUndo() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.swipeThread(swipedThread(), SwipeAction.DELETE)
            assertEquals(0, vm.state.value.confirmDeleteForGood)
            assertEquals(MessageAction.Delete, core.bulkCalls.single().second)
            assertEquals(BulkUndoKind.DELETED, vm.bulkUndoOffers.first().kind)
        }

    @Test
    fun aMoveSwipeOnAConversationMovesItsMessagesWithUndo() =
        runTest {
            val core = FakeMailbox(stored = 3)
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.swipeThread(swipedThread(), SwipeAction.MOVE)
            assertTrue("the screen asks for the folder first", core.bulkCalls.isEmpty())
            vm.moveThread(4, 9)
            val (selection, action, _) = core.bulkCalls.single()
            assertEquals(listOf(4L), selection.threadIds)
            assertEquals(MessageAction.Move(9), action)
            assertEquals(BulkUndoKind.MOVED, vm.bulkUndoOffers.first().kind)
        }

    /** #560 review: a reload (new mail, a sync) doesn't close a swiped conversation's "Delete for good?". */
    @Test
    fun aSwipedConversationsDeleteQuestionOutlivesAReload() =
        runTest {
            val core = FakeMailbox(stored = 3).apply { forGood = 3 }
            val vm = MessageListViewModel(core, FakeAppSettings(), FOLDER)
            vm.swipeThread(swipedThread(), SwipeAction.DELETE)
            assertEquals(3, vm.state.value.confirmDeleteForGood)
            core.events.tryEmit(CoreEvent.MessagesChanged(FOLDER))
            assertEquals("still asking after the reload", 3, vm.state.value.confirmDeleteForGood)
            // A second swiped delete while the first waits doesn't replace it.
            vm.swipeThread(swipedThread().copy(threadId = 6), SwipeAction.DELETE)
            vm.confirmDeleteForGood()
            assertEquals(listOf(4L), core.bulkCalls.single().first.threadIds)
            assertEquals(0, vm.state.value.confirmDeleteForGood)
        }
}
