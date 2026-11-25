package app.mailcrane.security

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppLockStateTest {
    private var clock = 1_000_000L
    private val state = AppLockState { clock }

    private fun away(ms: Long) {
        state.onBackground()
        clock += ms
        state.onForeground()
    }

    @Test
    fun `off by default and never locks`() {
        state.configure(enabled = false, timeoutMs = 0, coldStart = true)
        assertFalse(state.locked)
        away(10_000_000)
        assertFalse(state.locked)
    }

    @Test
    fun `cold start with the lock on is locked, so process death can't skip it`() {
        state.configure(enabled = true, timeoutMs = 3_600_000, coldStart = true)
        assertTrue(state.locked)
    }

    @Test
    fun `turning the lock on in a running app doesn't lock it`() {
        state.configure(enabled = false, timeoutMs = 60_000, coldStart = true)
        state.configure(enabled = true, timeoutMs = 60_000, coldStart = false)
        assertFalse(state.locked)
    }

    @Test
    fun `immediately locks on any return`() {
        unlockedWith(timeoutMs = 0)
        away(1)
        assertTrue(state.locked)
    }

    @Test
    fun `every timeout choice locks at the limit and not before`() {
        for (seconds in APP_LOCK_TIMEOUT_CHOICES.filter { it > 0 }) {
            val ms = seconds * 1_000L
            unlockedWith(ms)
            away(ms - 1)
            assertFalse("$seconds s: just under", state.locked)
            away(ms)
            assertTrue("$seconds s: at the limit", state.locked)
        }
    }

    @Test
    fun `short trips don't add up`() {
        unlockedWith(timeoutMs = 60_000)
        repeat(5) { away(50_000) }
        assertFalse(state.locked)
    }

    @Test
    fun `the unlock prompt sending the app to the background doesn't lock it again`() {
        state.configure(enabled = true, timeoutMs = 0, coldStart = true)
        state.beginAuthentication()
        away(5_000) // the device PIN screen covers the app
        state.endAuthentication(success = true)
        assertFalse(state.locked)
        // The prompt's own return to the foreground comes after it closed.
        state.onForeground()
        assertFalse(state.locked)
    }

    @Test
    fun `a cancelled prompt stays locked`() {
        state.configure(enabled = true, timeoutMs = 0, coldStart = true)
        state.beginAuthentication()
        state.endAuthentication(success = false)
        assertTrue(state.locked)
    }

    @Test
    fun `turning the lock off unlocks`() {
        state.configure(enabled = true, timeoutMs = 0, coldStart = true)
        state.configure(enabled = false, timeoutMs = 0, coldStart = false)
        assertFalse(state.locked)
    }

    @Test
    fun `a clock that goes backwards locks`() {
        unlockedWith(timeoutMs = 3_600_000)
        state.onBackground()
        clock -= 10
        state.onForeground()
        assertTrue(state.locked)
    }

    private fun unlockedWith(timeoutMs: Long) {
        state.configure(enabled = true, timeoutMs = timeoutMs, coldStart = true)
        state.beginAuthentication()
        state.endAuthentication(success = true)
        assertFalse(state.locked)
    }
}
