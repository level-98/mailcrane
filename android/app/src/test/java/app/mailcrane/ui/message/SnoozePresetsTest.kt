package app.mailcrane.ui.message

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import java.time.ZonedDateTime

class SnoozePresetsTest {
    private val london = ZoneId.of("Europe/London")
    private val newYork = ZoneId.of("America/New_York")

    private fun at(
        text: String,
        zone: ZoneId,
    ): ZonedDateTime = ZonedDateTime.of(java.time.LocalDateTime.parse(text), zone)

    private fun utc(
        preset: SnoozePreset,
        now: ZonedDateTime,
    ): Instant = snoozeTime(preset, now).toInstant()

    private data class Case(
        val now: ZonedDateTime,
        val preset: SnoozePreset,
        val expected: String,
    )

    @Test
    fun presetsAcrossTimeZonesAndDaylightSaving() {
        val cases =
            listOf(
                // The night before the clocks go forward in London (29 March 2026, 01:00 -> 02:00).
                Case(at("2026-03-28T22:00", london), SnoozePreset.TOMORROW, "2026-03-29T07:00:00Z"),
                Case(at("2026-03-28T22:00", london), SnoozePreset.LATER_TODAY, "2026-03-29T01:00:00Z"),
                // 00:30 GMT + 3 real hours = 04:30 BST, rounded up to 05:00 BST.
                Case(at("2026-03-29T00:30", london), SnoozePreset.LATER_TODAY, "2026-03-29T04:00:00Z"),
                Case(at("2026-03-28T22:00", london), SnoozePreset.NEXT_WEEK, "2026-03-30T07:00:00Z"),
                // Saturday evening: the weekend preset means next Saturday.
                Case(at("2026-03-28T22:00", london), SnoozePreset.THIS_WEEKEND, "2026-04-04T07:00:00Z"),
                // The night before the clocks go back in London (25 October 2026).
                Case(at("2026-10-24T23:00", london), SnoozePreset.TOMORROW, "2026-10-25T08:00:00Z"),
                // The night before New York springs forward (8 March 2026).
                Case(at("2026-03-07T23:00", newYork), SnoozePreset.TOMORROW, "2026-03-08T12:00:00Z"),
                Case(at("2026-03-07T23:00", newYork), SnoozePreset.LATER_TODAY, "2026-03-08T07:00:00Z"),
                // And before it falls back (1 November 2026).
                Case(at("2026-10-31T21:15", newYork), SnoozePreset.TOMORROW, "2026-11-01T13:00:00Z"),
                Case(at("2026-10-31T21:15", newYork), SnoozePreset.NEXT_WEEK, "2026-11-02T13:00:00Z"),
                // A Wednesday afternoon in winter.
                Case(at("2026-11-25T14:00", london), SnoozePreset.LATER_TODAY, "2026-11-25T17:00:00Z"),
                Case(at("2026-11-25T14:00", london), SnoozePreset.THIS_WEEKEND, "2026-11-28T08:00:00Z"),
                Case(at("2026-11-25T14:00", london), SnoozePreset.NEXT_WEEK, "2026-11-30T08:00:00Z"),
                // Saturday early morning: this weekend is today at 08:00.
                Case(at("2026-11-28T06:30", london), SnoozePreset.THIS_WEEKEND, "2026-11-28T08:00:00Z"),
                // Monday: next week is the following Monday, never today.
                Case(at("2026-11-30T06:00", london), SnoozePreset.NEXT_WEEK, "2026-12-07T08:00:00Z"),
                // Late evening: "later today" runs into tomorrow.
                Case(at("2026-11-25T22:10", newYork), SnoozePreset.LATER_TODAY, "2026-11-26T07:00:00Z"),
            )
        for (case in cases) {
            assertEquals("${case.preset} from ${case.now}", Instant.parse(case.expected), utc(case.preset, case.now))
        }
    }

    @Test
    fun laterTodayOnTheHourIsNotRoundedUp() {
        assertEquals(
            at("2026-11-25T17:00", london),
            snoozeTime(SnoozePreset.LATER_TODAY, at("2026-11-25T14:00", london)),
        )
    }
}
