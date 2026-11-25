package app.mailcrane.ui.theme

import androidx.compose.ui.graphics.ImageBitmap
import app.mailcrane.contacts.ContactPhotos
import app.mailcrane.contacts.NoContactPhotos
import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.MailboxDensity
import app.mailcrane.settings.ThemeMode
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ThemeViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun followsTheAppearanceSettings() =
        runTest {
            val settings = FakeAppSettings()
            val vm = ThemeViewModel(settings, FakePhotos)
            assertEquals(Appearance(), vm.appearance.value)
            settings.setThemeMode(ThemeMode.DARK)
            settings.setDynamicColor(false)
            settings.setDarkMessageBodies(true)
            val expected = Appearance(ThemeMode.DARK, dynamicColor = false, darkMessageBodies = true)
            assertEquals(expected, vm.appearance.value)
        }

    @Test
    fun densityAndContactPhotosFollowTheSettings() =
        runTest {
            val settings = FakeAppSettings()
            val vm = ThemeViewModel(settings, FakePhotos)
            assertEquals(MailboxDensity.COMFORTABLE, vm.mailboxDensity.value)
            // Off by default: no contact is read, whatever the permission.
            assertSame(NoContactPhotos, vm.contactPhotos.value)
            settings.setMailboxDensity(MailboxDensity.COMPACT)
            settings.setContactPhotos(true)
            assertEquals(MailboxDensity.COMPACT, vm.mailboxDensity.value)
            assertSame(FakePhotos, vm.contactPhotos.value)
            settings.setContactPhotos(false)
            assertSame(NoContactPhotos, vm.contactPhotos.value)
        }
}

private object FakePhotos : ContactPhotos {
    override suspend fun photo(address: String): ImageBitmap? = null
}
