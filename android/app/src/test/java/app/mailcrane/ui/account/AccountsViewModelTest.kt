package app.mailcrane.ui.account

import app.mailcrane.core.AccountSummary
import app.mailcrane.core.ConnectionSecurity
import app.mailcrane.core.MailServer
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.ManualAccountConfig
import app.mailcrane.core.RemoveAccountFailure
import app.mailcrane.settings.FakeAppSettings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
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
}
