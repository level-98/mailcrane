package app.mailcrane.ui.conversation

import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.NO_FILTERS
import app.mailcrane.core.SearchFlag
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.SwipeAction
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
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private const val THREAD = 5L
private const val ACCOUNT = 3L

/** A conversation of two messages; records swipes and the accounts whose folders were asked for. */
private class FakeThreadCore : MailboxCore {
    override val events = MutableSharedFlow<CoreEvent>()
    val swiped = mutableListOf<Triple<Long, MessageAction, Boolean>>()
    val foldersAsked = mutableListOf<Long>()
    var actionFailure: ActionFailure? = null
    var folders = emptyList<FolderSummary>()

    override suspend fun listThreadMessages(threadId: Long) =
        if (threadId == THREAD) listOf(message(1), message(2)) else emptyList()

    override suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    ) {
        actionFailure?.let { throw it }
        swiped += Triple(messageId, action, undoable)
    }

    override suspend fun undoAction(messageId: Long) = true

    override suspend fun listFolders(accountId: Long): List<FolderSummary> {
        foldersAsked += accountId
        return folders
    }

    override suspend fun listMessages(
        folderId: Long,
        offset: Int,
        limit: Int,
    ) = emptyList<MessageSummary>()

    override suspend fun syncFolder(folderId: Long) = Unit

    override suspend fun loadOlder(
        folderId: Long,
        count: Int,
    ) = 0

    override suspend fun refreshFolders(accountId: Long): Long? = null

    override suspend fun searchMessages(
        accountId: Long,
        folderId: Long?,
        query: String,
        limit: Int,
    ) = emptyList<MessageSummary>()

    override suspend fun searchServer(
        folderId: Long,
        query: String,
    ) = 0

    override suspend fun searchAllAccounts(
        query: String,
        limit: Int,
    ) = emptyList<MessageSummary>()

    override fun searchFilters(query: String) = NO_FILTERS

    override fun searchWithFlag(
        query: String,
        flag: SearchFlag,
        on: Boolean,
    ) = query

    override fun searchWithoutFrom(query: String) = query

    override fun searchWithAccount(
        query: String,
        account: String?,
    ) = query
}

private fun message(id: Long) =
    MessageSummary(id, "Ana", "ana@example.org", "Lunch", "", 1_000 + id, true, false, false, accountId = ACCOUNT)

/** Swipes on the messages inside a conversation (#552). */
@OptIn(ExperimentalCoroutinesApi::class)
class ConversationViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun anArchiveSwipeRunsThroughTheCoreAndOffersUndo() =
        runTest {
            val core = FakeThreadCore()
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(message(2), SwipeAction.ARCHIVE)
            assertEquals(listOf(Triple(2L, MessageAction.Archive, true)), core.swiped)
            val offer = vm.undoOffers.first()
            assertEquals(2L, offer.messageId)
            assertEquals(SwipeAction.ARCHIVE, offer.action)
        }

    @Test
    fun readAndFlagSwipesGoByTheMessage() =
        runTest {
            val core = FakeThreadCore()
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(message(1), SwipeAction.READ_UNREAD)
            vm.swipe(message(1).copy(isFlagged = true), SwipeAction.FLAG)
            assertEquals(
                listOf(Triple(1L, MessageAction.MarkUnread, false), Triple(1L, MessageAction.Unflag, false)),
                core.swiped,
            )
        }

    @Test
    fun aMoveSwipeOffersTheMessagesAccountFoldersThenMoves() =
        runTest {
            val core = FakeThreadCore()
            core.folders =
                listOf(
                    FolderSummary(10, "INBOX", null, 0u, 0u, canTakeMoves = true),
                    FolderSummary(11, "Projects", null, 0u, 0u, canTakeMoves = true),
                    FolderSummary(12, "Outbox", null, 0u, 0u, canTakeMoves = false),
                )
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(message(2), SwipeAction.MOVE)
            assertEquals(listOf(ACCOUNT), core.foldersAsked)
            val (messageId, offered) = vm.moveTargets.value!!
            assertEquals(2L, messageId)
            assertEquals("only folders that take moves", listOf(10L, 11L), offered.map { it.id })
            assertTrue("nothing moved before a pick", core.swiped.isEmpty())
            vm.move(11)
            assertEquals(listOf(Triple(2L, MessageAction.Move(11), false)), core.swiped)
            assertNull("the dialog closes", vm.moveTargets.value)
        }

    @Test
    fun aRefusedSwipeSaysWhy() =
        runTest {
            val core = FakeThreadCore().apply { actionFailure = ActionFailure.NoArchiveFolder() }
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(message(1), SwipeAction.ARCHIVE)
            assertEquals(R.string.action_no_archive_folder, vm.notices.first())
        }

    /** #504: a swipe on a message coming back from snooze says so. */
    @Test
    fun aSwipeOnAWakingMessageSaysItIsComingBack() =
        runTest {
            val core = FakeThreadCore().apply { actionFailure = ActionFailure.Waking() }
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(message(1), SwipeAction.DELETE)
            assertEquals(R.string.action_waking, vm.notices.first())
        }
}
