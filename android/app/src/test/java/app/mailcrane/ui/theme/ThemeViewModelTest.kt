package app.mailcrane.ui.theme

import app.mailcrane.settings.FakeAppSettings
import app.mailcrane.settings.ThemeMode
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
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
            val vm = ThemeViewModel(settings)
            assertEquals(Appearance(), vm.appearance.value)
            settings.setThemeMode(ThemeMode.DARK)
            settings.setDynamicColor(false)
            settings.setDarkMessageBodies(true)
            val expected = Appearance(ThemeMode.DARK, dynamicColor = false, darkMessageBodies = true)
            assertEquals(expected, vm.appearance.value)
        }
}
