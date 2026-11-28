package app.mailcrane.ui.conversation

import app.mailcrane.R
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.NO_FILTERS
import app.mailcrane.core.SearchFlag
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.SwipeAction
import app.mailcrane.ui.messagelist.UndoOffer
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

internal const val THREAD = 5L
internal const val ACCOUNT = 3L
internal const val INBOX = 10L
internal const val ARCHIVE = 11L
internal const val PROJECTS = 12L

/**
 * A conversation of two messages in the Inbox, as the core lists it: each message with its real account
 * and folder (#571). Records swipes and the accounts whose folders were read.
 */
internal class FakeThreadCore : MailboxCore {
    override val events = MutableSharedFlow<CoreEvent>(extraBufferCapacity = 16)
    var thread = listOf(threadMessage(1), threadMessage(2))
    val swiped = mutableListOf<Triple<Long, MessageAction, Boolean>>()
    val foldersAsked = mutableListOf<Long>()
    var actionFailure: ActionFailure? = null

    /** A delete swipe takes the message out of the conversation, as moving it to Trash does. */
    var deleteRemoves = false

    val folders =
        listOf(
            FolderSummary(INBOX, "INBOX", FolderRole.INBOX, 0u, 0u, canTakeMoves = true),
            FolderSummary(ARCHIVE, "Archive", FolderRole.ARCHIVE, 0u, 0u, canTakeMoves = true),
            FolderSummary(PROJECTS, "Projects", null, 0u, 0u, canTakeMoves = true),
            FolderSummary(13, "Outbox", null, 0u, 0u, canTakeMoves = false),
        )

    override suspend fun listThreadMessages(threadId: Long) = if (threadId == THREAD) thread else emptyList()

    override suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    ) {
        actionFailure?.let { throw it }
        swiped += Triple(messageId, action, undoable)
        if (deleteRemoves && action == MessageAction.Delete) {
            thread = thread.filter { it.id != messageId }
            events.tryEmit(CoreEvent.MessagesChanged(INBOX))
        }
    }

    override suspend fun undoAction(messageId: Long) = true

    override suspend fun listFolders(accountId: Long): List<FolderSummary> {
        foldersAsked += accountId
        return if (accountId == ACCOUNT) folders else emptyList()
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

internal fun threadMessage(
    id: Long,
    folder: Long = INBOX,
) = MessageSummary(
    id = id,
    fromName = "Ana",
    fromAddr = "ana@example.org",
    subject = "Lunch",
    snippet = "",
    date = 1_000 + id,
    isSeen = true,
    isFlagged = false,
    hasAttachments = false,
    accountId = ACCOUNT,
    folderId = folder,
)

/** Swipes on the messages inside a conversation (#571). */
@OptIn(ExperimentalCoroutinesApi::class)
class ConversationViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun aDeleteSwipeRunsThroughTheCoreAndOffersUndoOnce() =
        runTest {
            val core = FakeThreadCore()
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(threadMessage(2), SwipeAction.DELETE)
            assertEquals(listOf(Triple(2L, MessageAction.Delete, true)), core.swiped)
            val offer = vm.undo.value.offer
            assertEquals(UndoOffer(2, SwipeAction.DELETE), offer)
            assertEquals("nothing still waiting for the core", 0, vm.undo.value.inFlight)
            vm.offerDone(offer!!)
            assertNull("shown once, then gone: nothing left for the next opening", vm.undo.value.offer)
        }

    @Test
    fun readAndFlagSwipesGoByTheMessage() =
        runTest {
            val core = FakeThreadCore()
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(threadMessage(1), SwipeAction.READ_UNREAD)
            vm.swipe(threadMessage(1).copy(isFlagged = true), SwipeAction.FLAG)
            assertEquals(
                listOf(Triple(1L, MessageAction.MarkUnread, false), Triple(1L, MessageAction.Unflag, false)),
                core.swiped,
            )
            assertNull("no Undo for read and flag", vm.undo.value.offer)
        }

    /** Criterion 1: the picker lists the message's real account's folders, not an empty list. */
    @Test
    fun aMoveSwipeOffersTheMessagesAccountFoldersExceptItsOwnThenMoves() =
        runTest {
            val core = FakeThreadCore()
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(threadMessage(2), SwipeAction.MOVE)
            assertEquals(listOf(ACCOUNT), core.foldersAsked)
            val (messageId, offered) = vm.moveTargets.value!!
            assertEquals(2L, messageId)
            assertEquals(
                "folders that take moves, without the message's own (the Inbox)",
                listOf(ARCHIVE, PROJECTS),
                offered.map { it.id },
            )
            assertTrue("nothing moved before a pick", core.swiped.isEmpty())
            vm.move(PROJECTS)
            assertEquals(listOf(Triple(2L, MessageAction.Move(PROJECTS), false)), core.swiped)
            assertNull("the dialog closes", vm.moveTargets.value)
        }

    /** Criterion 2: archive moves a message that isn't archived, and only then says "Archived". */
    @Test
    fun archiveMovesAnInboxMessageAndLeavesAnArchivedOneAlone() =
        runTest {
            val core = FakeThreadCore()
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(threadMessage(1, folder = ARCHIVE), SwipeAction.ARCHIVE)
            assertEquals(R.string.conversation_already_archived, vm.notices.first())
            assertTrue("the core isn't asked", core.swiped.isEmpty())
            assertNull("no \"Archived\"", vm.undo.value.offer)

            vm.swipe(threadMessage(2), SwipeAction.ARCHIVE)
            assertEquals(listOf(Triple(2L, MessageAction.Archive, true)), core.swiped)
            assertEquals(UndoOffer(2, SwipeAction.ARCHIVE), vm.undo.value.offer)
            assertEquals("the account's folders are read once", listOf(ACCOUNT), core.foldersAsked)
        }

    @Test
    fun aRefusedSwipeSaysWhyAndOffersNoUndo() =
        runTest {
            val core = FakeThreadCore().apply { actionFailure = ActionFailure.NoArchiveFolder() }
            val vm = ConversationViewModel(core, FakeAppSettings(), THREAD)
            vm.swipe(threadMessage(1), SwipeAction.ARCHIVE)
            assertEquals(R.string.action_no_archive_folder, vm.notices.first())
            assertNull(vm.undo.value.offer)
            assertEquals(0, vm.undo.value.inFlight)
        }
}
