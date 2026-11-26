package app.mailcrane.ui.account

import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import app.mailcrane.core.RemoveAccountFailure
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.StartView
import app.mailcrane.core.UNIFIED_FOLDER_ID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private class FakeAccounts(
    var accounts: MutableList<AccountSummary>,
) : MailcraneCore {
    var failRemove = false

    override fun hello(): String = ""

    override fun defaultPort(
        protocol: MailServer,
        security: ConnectionSecurity,
    ): Int = 0

    override suspend fun addAccountManual(
        config: ManualAccountConfig,
        password: String,
    ): Long = 0

    override suspend fun listAccounts(): List<AccountSummary> = accounts.toList()

    override suspend fun removeAccount(accountId: Long) {
        if (failRemove) throw RemoveAccountFailure.Storage()
        accounts.removeAll { it.id == accountId }
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class AccountsViewModelTest {
    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    private fun acct(id: Long) = AccountSummary(id, "a$id@example.org", "A$id")

    @Test
    fun theLastAccountIsOpenedAgain() =
        runTest {
            val settings = FakeAppSettings().apply { setLastAccountId(2) }
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(1), acct(2))), settings)
            assertEquals(2L, vm.state.value.currentId)
            vm.select(1)
            assertEquals(1L, vm.state.value.currentId)
            assertEquals(1L, settings.lastAccountId.value)
            vm.select(99)
            assertEquals("unknown accounts are ignored", 1L, vm.state.value.currentId)
        }

    @Test
    fun withoutAMemoryTheFirstAccountOpens() =
        runTest {
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(3), acct(4))), FakeAppSettings())
            assertEquals(3L, vm.state.value.currentId)
            val empty = AccountsViewModel(FakeAccounts(mutableListOf()), FakeAppSettings())
            assertEquals(emptyList<AccountSummary>(), empty.state.value.accounts)
            assertEquals(null, empty.state.value.currentId)
        }

    @Test
    fun addingOpensTheNewAccountAndRemovingFallsBack() =
        runTest {
            val core = FakeAccounts(mutableListOf(acct(1)))
            val vm = AccountsViewModel(core, FakeAppSettings())
            core.accounts.add(acct(5))
            vm.added(5)
            assertEquals(5L, vm.state.value.currentId)
            vm.remove(5)
            assertEquals(1L, vm.state.value.currentId)
            assertEquals(
                listOf(1L),
                vm.state.value.accounts
                    ?.map { it.id },
            )
            core.failRemove = true
            vm.remove(1)
            assertTrue(vm.state.value.removeFailed)
            assertEquals(1L, vm.state.value.currentId)
        }

    // M4-16: "Open the app on".

    @Test
    fun lastUsedIsTheDefaultAndBringsBackTheUnifiedInbox() =
        runTest {
            val settings = FakeAppSettings().apply { setLastAccountId(2) }
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(1), acct(2))), settings)
            assertEquals(2L, vm.state.value.currentId)
            assertFalse(vm.state.value.startInUnified)

            settings.setLastFolderId(UNIFIED_FOLDER_ID)
            val again = AccountsViewModel(FakeAccounts(mutableListOf(acct(1), acct(2))), settings)
            assertEquals(2L, again.state.value.currentId)
            assertTrue(again.state.value.startInUnified)
            again.startShown()
            assertFalse("only at start", again.state.value.startInUnified)
        }

    @Test
    fun theUnifiedInboxChoiceOpensIt() =
        runTest {
            val settings = FakeAppSettings().apply { start.value = StartView.Unified }
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(1), acct(2))), settings)
            assertTrue(vm.state.value.startInUnified)
        }

    @Test
    fun aChosenAccountOpensWhateverWasShownLast() =
        runTest {
            val settings =
                FakeAppSettings().apply {
                    start.value = StartView.Account(1)
                    setLastAccountId(2)
                    setLastFolderId(UNIFIED_FOLDER_ID)
                }
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(1), acct(2))), settings)
            assertEquals(1L, vm.state.value.currentId)
            assertFalse(vm.state.value.startInUnified)
        }

    @Test
    fun aRemovedChosenAccountFallsBackToTheFirstAndLastUsed() =
        runTest {
            val settings = FakeAppSettings().apply { start.value = StartView.Account(9) }
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(3), acct(4))), settings)
            assertEquals(3L, vm.state.value.currentId)
            assertEquals(StartView.Last, settings.start.value)
        }

    @Test
    fun removingTheChosenAccountResetsTheSetting() =
        runTest {
            val settings = FakeAppSettings().apply { start.value = StartView.Account(4) }
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(3), acct(4))), settings)
            assertEquals(4L, vm.state.value.currentId)
            vm.remove(4)
            assertEquals(3L, vm.state.value.currentId)
            assertEquals(StartView.Last, settings.start.value)
        }

    @Test
    fun theShownFolderIsRemembered() =
        runTest {
            val settings = FakeAppSettings()
            val vm = AccountsViewModel(FakeAccounts(mutableListOf(acct(1))), settings)
            vm.folderShown(UNIFIED_FOLDER_ID)
            assertEquals(UNIFIED_FOLDER_ID, settings.lastFolder.value)
        }

    @Test
    fun startChoiceCoversEveryCase() {
        val ids = listOf(1L, 2L)
        assertEquals(StartChoice(2, unified = false), startChoice(StartView.Last, ids, 2, 7))
        assertEquals(StartChoice(1, unified = false), startChoice(StartView.Last, ids, 99, 7))
        assertEquals(StartChoice(2, unified = true), startChoice(StartView.Last, ids, 2, UNIFIED_FOLDER_ID))
        assertEquals(StartChoice(2, unified = true), startChoice(StartView.Unified, ids, 2, 7))
        assertEquals(StartChoice(1, unified = false), startChoice(StartView.Account(1), ids, 2, UNIFIED_FOLDER_ID))
        assertEquals(StartChoice(1, false, resetToLast = true), startChoice(StartView.Account(5), ids, 2, 7))
        assertEquals(StartChoice(null, unified = false), startChoice(StartView.Unified, emptyList(), 0, 0))
    }
}
