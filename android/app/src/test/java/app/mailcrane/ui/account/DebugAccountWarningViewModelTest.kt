package app.mailcrane.ui.account

import app.mailcrane.debug.DebugTools
import app.mailcrane.debug.NoDebugTools
import app.mailcrane.settings.FakeAppSettings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private object AvailableDebugTools : DebugTools {
    override val available: Boolean = true

    override suspend fun loadFixtures(): Int = 0
}

@OptIn(ExperimentalCoroutinesApi::class)
class DebugAccountWarningViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun aDebugBuildWarnsOnceUntilDismissed() {
        val settings = FakeAppSettings()
        val vm = DebugAccountWarningViewModel(AvailableDebugTools, settings)
        assertTrue(vm.show.value)
        vm.dismiss()
        assertFalse(vm.show.value)
        assertTrue(settings.debugWarningSeen.value)
        // Seen before: a new setup screen doesn't show it again.
        assertFalse(DebugAccountWarningViewModel(AvailableDebugTools, settings).show.value)
    }

    @Test
    fun aReleaseBuildNeverWarns() {
        // Release builds bind NoDebugTools (src/release DebugToolsModule).
        val vm = DebugAccountWarningViewModel(NoDebugTools, FakeAppSettings())
        assertFalse(vm.show.value)
    }
}
