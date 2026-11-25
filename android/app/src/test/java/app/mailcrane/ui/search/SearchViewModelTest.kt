package app.mailcrane.ui.search

import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.NO_FILTERS
import app.mailcrane.core.SearchFilters
import app.mailcrane.core.SearchFlag
import app.mailcrane.core.UNIFIED_FOLDER_ID
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
import org.junit.Assert.assertTrue
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

    var everywhere = mapOf<String, List<MessageSummary>>()
    var everywhereAsked = 0

    override suspend fun searchAllAccounts(
        query: String,
        limit: Int,
    ): List<MessageSummary> {
        everywhereAsked++
        return everywhere[query].orEmpty()
    }

    // A tiny stand-in for the core's query editing: whole tokens only.
    private fun tokens(query: String) = query.split(" ").filter { it.isNotBlank() }

    override fun searchFilters(query: String): SearchFilters {
        val t = tokens(query)
        return NO_FILTERS.copy(
            from = t.filter { it.startsWith("from:") && it.length > 5 }.map { it.removePrefix("from:") },
            hasAttachment = "has:attachment" in t,
            unread = "is:unread" in t,
            flagged = "is:flagged" in t,
            account = t.lastOrNull { it.startsWith("account:") }?.removePrefix("account:"),
        )
    }

    private fun with(
        query: String,
        drop: (String) -> Boolean,
        add: String?,
    ) = (tokens(query).filterNot(drop) + listOfNotNull(add)).joinToString(" ")

    override fun searchWithFlag(
        query: String,
        flag: SearchFlag,
        on: Boolean,
    ): String {
        val token =
            when (flag) {
                SearchFlag.HAS_ATTACHMENT -> "has:attachment"
                SearchFlag.UNREAD -> "is:unread"
                SearchFlag.FLAGGED -> "is:flagged"
            }
        return with(query, { it == token }, token.takeIf { on })
    }

    override fun searchWithoutFrom(query: String) = with(query, { it.startsWith("from:") }, null)

    override fun searchWithAccount(
        query: String,
        account: String?,
    ) = with(query, { it.startsWith("account:") }, account?.let { "account:$it" })
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

    @Test
    fun chipsEditTheQueryAndSearchWithIt() {
        val core = FakeSearch().apply { local = mapOf("lunch is:unread" to listOf(summary(4))) }
        val vm = SearchViewModel(core, FakeAppSettings(), 1, 7)
        vm.onQueryChange("lunch")
        vm.onFlag(SearchFlag.UNREAD, true)
        assertEquals("lunch is:unread", vm.state.value.query)
        assertTrue(vm.state.value.filters.unread)
        scheduler.advanceTimeBy(300)
        scheduler.runCurrent()
        assertEquals(listOf(4L), vm.state.value.results?.map { it.id })
        vm.onFlag(SearchFlag.UNREAD, false)
        assertEquals("lunch", vm.state.value.query)
        assertFalse(vm.state.value.filters.unread)
        vm.onAccount("ann@example.net")
        assertEquals("ann@example.net", vm.state.value.filters.account)
        vm.onAccount(null)
        assertEquals("lunch", vm.state.value.query)
    }

    @Test
    fun theFromChipStartsATermToTypeIntoAndRemovesFilledOnes() {
        val vm = SearchViewModel(FakeSearch(), FakeAppSettings(), 1, 7)
        vm.onQueryChange("lunch")
        vm.onFrom()
        assertEquals("lunch from:", vm.state.value.query)
        vm.onQueryChange("lunch from:ana")
        assertEquals(listOf("ana"), vm.state.value.filters.from)
        vm.onFrom()
        assertEquals("lunch", vm.state.value.query)
    }

    @Test
    fun searchingFromTheUnifiedInboxCoversEveryAccountAndSkipsTheServer() {
        val core = FakeSearch().apply { everywhere = mapOf("lunch" to listOf(summary(2), summary(3))) }
        val vm = SearchViewModel(core, FakeAppSettings(), 1, UNIFIED_FOLDER_ID)
        vm.onQueryChange("lunch")
        vm.submit()
        scheduler.runCurrent()
        assertEquals(listOf(2L, 3L), vm.state.value.results?.map { it.id })
        assertTrue(core.everywhereAsked > 0)
        assertEquals(0, core.serverAsked)
        assertFalse(vm.state.value.searchingServer)
    }
}
