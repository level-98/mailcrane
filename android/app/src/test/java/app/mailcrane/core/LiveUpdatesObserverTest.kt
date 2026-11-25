package app.mailcrane.core

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class LiveUpdatesObserverTest {
    private class FakeEngine(
        private val ids: List<Long>,
        private val failing: Set<Long> = emptySet(),
    ) : LiveUpdatesEngine {
        val calls = mutableListOf<String>()

        override fun accountIds(): List<Long> = ids

        override fun start(accountId: Long): Boolean {
            calls += "start $accountId"
            return accountId !in failing
        }

        override fun stop(accountId: Long) {
            calls += "stop $accountId"
        }
    }

    @Test
    fun foregroundStartsEveryAccountAndBackgroundStopsThem() =
        runTest {
            val engine = FakeEngine(listOf(1, 2))
            val observer = LiveUpdatesObserver(engine, this, StandardTestDispatcher(testScheduler))

            observer.onForeground()
            advanceUntilIdle()
            assertEquals(listOf("start 1", "start 2"), engine.calls)

            observer.onBackground()
            advanceUntilIdle()
            assertEquals(listOf("start 1", "start 2", "stop 1", "stop 2"), engine.calls)
        }

    @Test
    fun aQuickSwitchBackToForegroundEndsRunning() =
        runTest {
            val engine = FakeEngine(listOf(7))
            val observer = LiveUpdatesObserver(engine, this, StandardTestDispatcher(testScheduler))

            observer.onForeground()
            observer.onBackground()
            observer.onForeground()
            advanceUntilIdle()

            // In the order the lifecycle changed, so updates end up running.
            assertEquals(listOf("start 7", "stop 7", "start 7"), engine.calls)
        }

    @Test
    fun anAccountThatFailedToStartIsNotStopped() =
        runTest {
            val engine = FakeEngine(listOf(1, 2), failing = setOf(2))
            val observer = LiveUpdatesObserver(engine, this, StandardTestDispatcher(testScheduler))

            observer.onForeground()
            observer.onBackground()
            advanceUntilIdle()

            assertEquals(listOf("start 1", "start 2", "stop 1"), engine.calls)
        }

    @Test
    fun backgroundWithoutForegroundDoesNothing() =
        runTest {
            val engine = FakeEngine(listOf(1))
            val observer = LiveUpdatesObserver(engine, this, StandardTestDispatcher(testScheduler))

            observer.onBackground()
            advanceUntilIdle()

            assertEquals(emptyList<String>(), engine.calls)
        }

    @Test
    fun alwaysOnSyncKeepsThemRunningInTheBackground() =
        runTest {
            val engine = FakeEngine(listOf(3))
            var alwaysOn = true
            val observer =
                LiveUpdatesObserver(engine, this, StandardTestDispatcher(testScheduler)) { alwaysOn }

            observer.onForeground()
            observer.onBackground()
            advanceUntilIdle()
            assertEquals(listOf("start 3"), engine.calls)

            // Turned off again: the next trip to the background stops them as usual.
            alwaysOn = false
            observer.onForeground()
            observer.onBackground()
            advanceUntilIdle()
            assertEquals(listOf("start 3", "start 3", "stop 3"), engine.calls)
        }
}
