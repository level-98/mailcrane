package app.mailcrane.sync

import app.mailcrane.core.BackgroundDownload
import app.mailcrane.settings.BackgroundDownloadSetting
import org.junit.Assert.assertEquals
import org.junit.Test

class BackgroundDownloadsTest {
    private val wifi = DeviceState(unmetered = true, batterySaver = false, freeBytes = 2_000_000_000L)

    @Test
    fun eachSettingMapsToTheCoreSetting() {
        assertEquals(
            BackgroundDownload.WIFI_ONLY,
            deviceConditions(BackgroundDownloadSetting.WIFI_ONLY, wifi).setting,
        )
        assertEquals(BackgroundDownload.ALWAYS, deviceConditions(BackgroundDownloadSetting.ALWAYS, wifi).setting)
        assertEquals(BackgroundDownload.NEVER, deviceConditions(BackgroundDownloadSetting.NEVER, wifi).setting)
    }

    @Test
    fun theDeviceStateIsPassedOnAsItIs() {
        val state = DeviceState(unmetered = false, batterySaver = true, freeBytes = 123L)
        val conditions = deviceConditions(BackgroundDownloadSetting.WIFI_ONLY, state)
        assertEquals(false, conditions.unmetered)
        assertEquals(true, conditions.batterySaver)
        assertEquals(123uL, conditions.freeBytes)
    }

    @Test
    fun unreadableFreeSpaceCountsAsNone() {
        val conditions = deviceConditions(BackgroundDownloadSetting.ALWAYS, wifi.copy(freeBytes = -1L))
        assertEquals(0uL, conditions.freeBytes)
    }
}
