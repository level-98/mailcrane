package app.mailcrane.settings

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
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

@OptIn(ExperimentalCoroutinesApi::class)
class SettingsViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun cleanLinksIsOnByDefaultAndCanBeTurnedOff() =
        runTest {
            val settings = FakeAppSettings()
            val vm = SettingsViewModel(settings)
            backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { vm.cleanLinks.collect {} }
            assertTrue(vm.cleanLinks.value)

            vm.setCleanLinks(false)
            assertFalse(vm.cleanLinks.value)
            assertFalse(settings.cleanLinks.value)

            vm.setCleanLinks(true)
            assertTrue(vm.cleanLinks.value)
        }

    @Test
    fun confirmLinksIsOnByDefaultAndCanBeTurnedOff() =
        runTest {
            val settings = FakeAppSettings()
            val vm = SettingsViewModel(settings)
            backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { vm.confirmLinks.collect {} }
            assertTrue(vm.confirmLinks.value)

            vm.setConfirmLinks(false)
            assertFalse(vm.confirmLinks.value)
            assertFalse(settings.confirmLinks.value)
        }

    @Test
    fun mailboxDensityIsComfortableByDefaultAndCanBeCompact() =
        runTest {
            val settings = FakeAppSettings()
            val vm = SettingsViewModel(settings)
            backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { vm.mailboxDensity.collect {} }
            assertEquals(MailboxDensity.COMFORTABLE, vm.mailboxDensity.value)

            vm.setMailboxDensity(MailboxDensity.COMPACT)
            assertEquals(MailboxDensity.COMPACT, vm.mailboxDensity.value)
            assertEquals(MailboxDensity.COMPACT, settings.density.value)
        }

    @Test
    fun contactPhotosAreOffByDefault() =
        runTest {
            val settings = FakeAppSettings()
            val vm = SettingsViewModel(settings)
            backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { vm.contactPhotos.collect {} }
            assertFalse(vm.contactPhotos.value)
            vm.setContactPhotos(true)
            assertTrue(settings.photos.value)
        }
}
