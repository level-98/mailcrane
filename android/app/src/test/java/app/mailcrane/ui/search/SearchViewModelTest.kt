package app.mailcrane.ui.search

import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.settings.FakeAppSettings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.TestCoroutineScheduler
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

private fun summary(id: Long) =
    MessageSummary(id, "Ana", "ana@example.org", "Subject $id", "", 1_000, false, false, false)

private class FakeSearch : MailboxCore {
    override val events = MutableSharedFlow<CoreEvent>()
    var local = mapOf<String, List<MessageSummary>>()
    var afterServer = mapOf<String, List<MessageSummary>>()
    var serverFailure: SyncErrorKind? = null
    var serverAsked = 0
    var serverDone = false

    override suspend fun searchMessages(
        accountId: Long,
        folderId: Long?,
        query: String,
        limit: Int,
    ): List<MessageSummary> = (if (serverDone) afterServer[query] else null) ?: local[query].orEmpty()

    override suspend fun searchServer(
        folderId: Long,
        query: String,
    ): Int {
        serverAsked++
        serverFailure?.let { throw SyncFailure.Failed(it) }
        serverDone = true
        return afterServer[query]?.size ?: 0
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

    override suspend fun listFolders(accountId: Long) = emptyList<FolderSummary>()

    override suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    ) = Unit

    override suspend fun undoAction(messageId: Long) = false
}

@OptIn(ExperimentalCoroutinesApi::class)
class SearchViewModelTest {
    private val scheduler = TestCoroutineScheduler()

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher(scheduler))

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun typingSearchesLocallyAfterAPause() {
        val core = FakeSearch().apply { local = mapOf("lun" to listOf(summary(1))) }
        val vm = SearchViewModel(core, FakeAppSettings(), 1, 7)
        vm.onQueryChange("lun")
        assertNull(vm.state.value.results)
        scheduler.advanceTimeBy(300)
        scheduler.runCurrent()
        assertEquals(
            listOf(1L),
            vm.state.value.results
                ?.map { it.id },
        )
        assertEquals(0, core.serverAsked)
    }

    @Test
    fun submittingAsksTheServerAndRemembersTheQuery() {
        val core =
            FakeSearch().apply {
                local = mapOf("zebra" to emptyList())
                afterServer = mapOf("zebra" to listOf(summary(9)))
            }
        val settings = FakeAppSettings()
        val vm = SearchViewModel(core, settings, 1, 7)
        vm.onQueryChange("zebra")
        vm.submit()
        scheduler.runCurrent()
        assertEquals(1, core.serverAsked)
        assertEquals(
            listOf(9L),
            vm.state.value.results
                ?.map { it.id },
        )
        assertFalse(vm.state.value.searchingServer)
        assertEquals(listOf("zebra"), settings.recentSearches.value)
    }

    @Test
    fun offlineKeepsTheLocalResults() {
        val core =
            FakeSearch().apply {
                local = mapOf("lunch" to listOf(summary(1)))
                serverFailure = SyncErrorKind.NETWORK
            }
        val vm = SearchViewModel(core, FakeAppSettings(), 1, 7)
        vm.onQueryChange("lunch")
        vm.submit()
        scheduler.runCurrent()
        assertEquals(
            listOf(1L),
            vm.state.value.results
                ?.map { it.id },
        )
        assertFalse(vm.state.value.searchingServer)
    }
}
