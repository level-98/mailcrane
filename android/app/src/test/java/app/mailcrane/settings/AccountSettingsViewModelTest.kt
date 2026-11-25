package app.mailcrane.settings

import app.mailcrane.core.AccountSettingsCore
import app.mailcrane.core.AccountSignature
import app.mailcrane.core.SaveSentSetting
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

internal class FakeSettings : AccountSettingsCore {
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

    var saveSent = SaveSentSetting.AUTOMATIC
    var saveSentFailure: SettingsFailure? = null

    override suspend fun getSaveSent(accountId: Long) = saveSent

    override suspend fun setSaveSent(
        accountId: Long,
        setting: SaveSentSetting,
    ) {
        saveSentFailure?.let { throw it }
        saveSent = setting
    }

    var color: Int? = null
    var colorFailure: SettingsFailure? = null

    override suspend fun getAccountColor(accountId: Long) = color

    override suspend fun setAccountColor(
        accountId: Long,
        color: Int?,
    ) {
        colorFailure?.let { throw it }
        this.color = color
    }

    override fun automaticAccountColor(accountId: Long) = 3

    var authservId: String? = null
    var authservFailure: SettingsFailure? = null

    override suspend fun getProviderAuthservId(accountId: Long) = authservId

    override suspend fun setProviderAuthservId(
        accountId: Long,
        value: String,
    ): String? {
        authservFailure?.let { throw it }
        // Like the core: trimmed and lowercased, empty clears, a space is refused.
        val cleaned = value.trim().lowercase().ifEmpty { null }
        if (cleaned?.contains(' ') == true) throw SettingsFailure.InvalidAuthservId()
        authservId = cleaned
        return cleaned
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

    @Test
    fun theSaveSentChoiceSavesAtOnceAndRevertsOnFailure() {
        val core = FakeSettings()
        val vm = AccountSettingsViewModel(core, 1)
        assertEquals(SaveSentSetting.AUTOMATIC, vm.state.value.saveSent)
        vm.onSaveSentChange(SaveSentSetting.OFF)
        assertEquals(SaveSentSetting.OFF, core.saveSent)
        assertEquals(SaveSentSetting.OFF, vm.state.value.saveSent)

        core.saveSentFailure = SettingsFailure.Storage()
        vm.onSaveSentChange(SaveSentSetting.ON)
        assertEquals(SaveSentSetting.OFF, vm.state.value.saveSent)
    }

    @Test
    fun choosesAndResetsTheAccountColourAndTellsTheLists() {
        val core = FakeSettings()
        val vm = AccountSettingsViewModel(core, 1)
        assertEquals(AccountColorState(chosen = null, automatic = 3), vm.state.value.color)
        assertEquals(
            3,
            vm.state.value.color
                ?.shown,
        )
        var refreshed = 0
        vm.onColorChange(5) { refreshed++ }
        assertEquals(5, core.color)
        assertEquals(
            5,
            vm.state.value.color
                ?.shown,
        )
        assertEquals(1, refreshed)
        vm.onColorChange(5) { refreshed++ }
        assertEquals("choosing the same colour again saves nothing", 1, refreshed)
        vm.onColorChange(null) { refreshed++ }
        assertEquals(null, core.color)
        assertEquals(
            3,
            vm.state.value.color
                ?.shown,
        )
        assertEquals(2, refreshed)
    }

    @Test
    fun aFailedColourSaveGoesBackAndTellsNoOne() {
        val core = FakeSettings().apply { color = 1 }
        val vm = AccountSettingsViewModel(core, 1)
        core.colorFailure = SettingsFailure.Storage()
        var refreshed = false
        vm.onColorChange(6) { refreshed = true }
        assertEquals(
            1,
            vm.state.value.color
                ?.chosen,
        )
        assertFalse(refreshed)
    }
}
