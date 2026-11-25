package app.mailcrane.sync

import org.junit.Assert.assertEquals
import org.junit.Test

class BackgroundModeTest {
    @Test
    fun notificationsOffMeansNothingRunsInTheBackground() {
        assertEquals(BackgroundMode.NONE, backgroundMode(notifications = false, alwaysOn = false))
        assertEquals(BackgroundMode.NONE, backgroundMode(notifications = false, alwaysOn = true))
    }

    @Test
    fun periodicUnlessAlwaysOnIsChosen() {
        assertEquals(BackgroundMode.PERIODIC, backgroundMode(notifications = true, alwaysOn = false))
        assertEquals(BackgroundMode.ALWAYS_ON, backgroundMode(notifications = true, alwaysOn = true))
    }
}
