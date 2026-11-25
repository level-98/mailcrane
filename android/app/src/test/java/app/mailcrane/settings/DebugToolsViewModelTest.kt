package app.mailcrane.settings

import app.mailcrane.debug.DebugTools
import app.mailcrane.debug.NoDebugTools
import kotlinx.coroutines.CompletableDeferred
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

private class FakeDebugTools(
    private val result: suspend () -> Int,
) : DebugTools {
    var calls = 0

    override val available: Boolean = true

    override suspend fun loadFixtures(): Int {
        calls++
        return result()
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class DebugToolsViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun releaseBuildsHaveNothingToShowOrRun() {
        val vm = DebugToolsViewModel(NoDebugTools)
        assertFalse(vm.available)
        var refreshed = false
        vm.loadFixtures { refreshed = true }
        assertEquals(TestMailState.Idle, vm.state.value)
        assertFalse(refreshed)
    }

    @Test
    fun loadingReportsTheCountAndRefreshesTheAccounts() =
        runTest {
            val gate = CompletableDeferred<Int>()
            val tools = FakeDebugTools { gate.await() }
            val vm = DebugToolsViewModel(tools)
            var refreshed = 0
            vm.loadFixtures { refreshed++ }
            assertEquals(TestMailState.Loading, vm.state.value)
            // A second tap while loading does nothing.
            vm.loadFixtures { refreshed++ }
            gate.complete(96)
            assertEquals(TestMailState.Loaded(96), vm.state.value)
            assertEquals(1, tools.calls)
            assertEquals(1, refreshed)
        }

    @Test
    fun aFailureIsShownAndNothingRefreshes() =
        runTest {
            val vm = DebugToolsViewModel(FakeDebugTools { throw IllegalStateException("storage") })
            var refreshed = false
            vm.loadFixtures { refreshed = true }
            assertEquals(TestMailState.Failed, vm.state.value)
            assertFalse(refreshed)
            assertTrue(vm.available)
        }
}
