package app.mailcrane.settings

import app.mailcrane.core.AccountSettingsCore
import app.mailcrane.core.AccountSignature
import app.mailcrane.core.SettingsFailure
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private class FakeSettings : AccountSettingsCore {
    var stored = AccountSignature("Ann", true)
    var failure: SettingsFailure? = null

    override suspend fun getSignature(accountId: Long): AccountSignature = stored

    override suspend fun setSignature(
        accountId: Long,
        signature: AccountSignature,
    ) {
        failure?.let { throw it }
        stored = signature
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class AccountSettingsViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun loadsEditsAndSavesTheSignature() {
        val core = FakeSettings()
        val vm = AccountSettingsViewModel(core, 1)
        assertEquals("Ann", vm.state.value.text)
        assertFalse(vm.state.value.dirty)

        vm.onTextChange("Ann\nExample Ltd")
        vm.onEnabledChange(false)
        assertTrue(vm.state.value.dirty)
        vm.save()
        assertEquals(AccountSignature("Ann\nExample Ltd", false), core.stored)
        assertEquals(SaveStatus.SAVED, vm.state.value.status)
        assertFalse(vm.state.value.dirty)
    }

    @Test
    fun aTooLongSignatureIsReportedAndKeptForEditing() {
        val core = FakeSettings().apply { failure = SettingsFailure.TooLong() }
        val vm = AccountSettingsViewModel(core, 1)
        vm.onTextChange("x".repeat(20))
        vm.save()
        assertEquals(SaveStatus.TOO_LONG, vm.state.value.status)
        assertTrue(vm.state.value.dirty)
        assertEquals(AccountSignature("Ann", true), core.stored)
    }
}
