package app.mailcrane.settings

import app.mailcrane.core.SenderTrustChanges
import app.mailcrane.core.SettingsFailure
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
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

@OptIn(ExperimentalCoroutinesApi::class)
class ProviderAuthservViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun emptyByDefaultThenSavedAsTheCoreCleanedIt() {
        val core = FakeSettings()
        val vm = ProviderAuthservViewModel(core, SenderTrustChanges(), 1)
        assertTrue(vm.state.value.loaded)
        assertEquals("", vm.state.value.text)
        assertFalse(vm.state.value.dirty)

        vm.onTextChange("  MX2.Example.org ")
        assertTrue(vm.state.value.dirty)
        vm.save()
        assertEquals("mx2.example.org", core.authservId)
        assertEquals("mx2.example.org", vm.state.value.text)
        assertEquals(AuthservSaveStatus.SAVED, vm.state.value.status)
        assertFalse(vm.state.value.dirty)
    }

    @Test
    fun clearingTheFieldClearsTheSetting() {
        val core = FakeSettings().apply { authservId = "mx2.example.org" }
        val vm = ProviderAuthservViewModel(core, SenderTrustChanges(), 1)
        assertEquals("mx2.example.org", vm.state.value.text)
        vm.onTextChange("")
        vm.save()
        assertNull(core.authservId)
        assertEquals(AuthservSaveStatus.CLEARED, vm.state.value.status)
    }

    @Test
    fun anInvalidNameIsReportedAndKeptForEditing() {
        val core = FakeSettings()
        val vm = ProviderAuthservViewModel(core, SenderTrustChanges(), 1)
        vm.onTextChange("mx 2.example.org")
        vm.save()
        assertEquals(AuthservSaveStatus.INVALID, vm.state.value.status)
        assertEquals("mx 2.example.org", vm.state.value.text)
        assertNull(core.authservId)
        assertTrue(vm.state.value.dirty)
    }

    @Test
    fun savingAndClearingTellOpenMessagesButARefusedNameDoesNot() {
        val core = FakeSettings()
        val trust = SenderTrustChanges()
        val vm = ProviderAuthservViewModel(core, trust, 1)
        vm.onTextChange("mx2.example.org")
        vm.save()
        assertEquals(1L, trust.version.value)
        vm.onTextChange("mx 2")
        vm.save()
        assertEquals(1L, trust.version.value)
        vm.onTextChange("")
        vm.save()
        assertEquals(2L, trust.version.value)
    }

    @Test
    fun aStorageFailureIsReported() {
        val core = FakeSettings().apply { authservFailure = SettingsFailure.Storage() }
        val vm = ProviderAuthservViewModel(core, SenderTrustChanges(), 1)
        vm.onTextChange("mx2.example.org")
        vm.save()
        assertEquals(AuthservSaveStatus.FAILED, vm.state.value.status)
    }
}
