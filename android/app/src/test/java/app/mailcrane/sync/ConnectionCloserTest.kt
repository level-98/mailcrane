package app.mailcrane.sync

import app.mailcrane.core.CloseReason
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import org.junit.Assert.assertEquals
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ConnectionCloserTest {
    private val scope = TestScope(StandardTestDispatcher())
    private val closed = mutableListOf<CloseReason>()
    private val closer = ConnectionCloser(scope, backgroundDelayMs = 60_000L) { closed += it }

    @Test
    fun aNewDefaultNetworkClosesButTheFirstOneDoesNot() {
        closer.onNetworkAvailable(1L)
        closer.onNetworkAvailable(1L)
        scope.runCurrent()
        assertEquals(emptyList<CloseReason>(), closed)
        closer.onNetworkAvailable(2L)
        scope.runCurrent()
        assertEquals(listOf(CloseReason.NETWORK_CHANGED), closed)
    }

    @Test
    fun losingTheDefaultNetworkCloses() {
        closer.onNetworkAvailable(1L)
        closer.onNetworkLost(7L)
        scope.runCurrent()
        assertEquals(emptyList<CloseReason>(), closed)
        closer.onNetworkLost(1L)
        scope.runCurrent()
        assertEquals(listOf(CloseReason.NETWORK_CHANGED), closed)
    }

    @Test
    fun aMinuteInTheBackgroundCloses() {
        closer.onBackground()
        scope.advanceTimeBy(59_000L)
        scope.runCurrent()
        assertEquals(emptyList<CloseReason>(), closed)
        scope.advanceTimeBy(2_000L)
        scope.runCurrent()
        assertEquals(listOf(CloseReason.BACKGROUND), closed)
    }

    @Test
    fun comingBackWithinTheMinuteKeepsThem() {
        closer.onBackground()
        scope.advanceTimeBy(30_000L)
        closer.onForeground()
        scope.advanceTimeBy(60_000L)
        scope.runCurrent()
        assertEquals(emptyList<CloseReason>(), closed)
    }
}
