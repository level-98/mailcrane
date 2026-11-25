package app.mailcrane.ui.compose

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.ZoneId
import java.time.ZonedDateTime

class SchedulePresetTest {
    private val zone = ZoneId.of("Europe/London")

    private fun at(text: String) = ZonedDateTime.parse(text).withZoneSameLocal(zone)

    @Test
    fun presetsLandOnTheRightLocalTimes() {
        // A Wednesday afternoon.
        val now = at("2026-11-25T15:30:00Z")
        assertEquals(at("2026-11-26T08:00:00Z").toEpochSecond(), presetTime(SchedulePreset.TOMORROW_MORNING, now))
        assertEquals(at("2026-11-26T13:00:00Z").toEpochSecond(), presetTime(SchedulePreset.TOMORROW_AFTERNOON, now))
        assertEquals(at("2026-11-30T08:00:00Z").toEpochSecond(), presetTime(SchedulePreset.MONDAY_MORNING, now))
    }

    @Test
    fun mondayOnAMondayMeansNextWeek() {
        val monday = at("2026-11-30T07:00:00Z")
        assertEquals(at("2026-12-07T08:00:00Z").toEpochSecond(), presetTime(SchedulePreset.MONDAY_MORNING, monday))
    }
}
