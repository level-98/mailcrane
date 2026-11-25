package app.mailcrane.ui.messagelist

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.DayOfWeek
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Locale

class DateHeadersTest {
    private val zone = ZoneId.of("Europe/London")

    // Wednesday 25 November 2026.
    private val today = LocalDate.of(2026, 11, 25)

    private fun at(
        date: LocalDate,
        hour: Int = 12,
        z: ZoneId = zone,
    ) = ZonedDateTime.of(LocalDateTime.of(date, LocalTime.of(hour, 0)), z).toEpochSecond()

    private fun group(
        epoch: Long,
        firstDay: DayOfWeek = DayOfWeek.MONDAY,
        on: LocalDate = today,
        z: ZoneId = zone,
    ) = dateGroup(epoch, on, z, firstDay)

    @Test
    fun eachGroup() {
        assertEquals(DateGroup.Today, group(at(today, 0)))
        assertEquals(DateGroup.Today, group(at(today, 23)))
        assertEquals(DateGroup.Yesterday, group(at(today.minusDays(1))))
        // Monday 23 November is in this week (UK weeks start on Monday).
        assertEquals(DateGroup.ThisWeek, group(at(LocalDate.of(2026, 11, 23))))
        // Sunday 22 November is last week: it goes under its month.
        assertEquals(DateGroup.Month(2026, 11), group(at(LocalDate.of(2026, 11, 22))))
        assertEquals(DateGroup.Month(2026, 10), group(at(LocalDate.of(2026, 10, 31))))
    }

    @Test
    fun weekStartFollowsTheLocale() {
        assertEquals(DayOfWeek.MONDAY, firstDayOfWeek(Locale.UK))
        assertEquals(DayOfWeek.SUNDAY, firstDayOfWeek(Locale.US))
        val sunday = at(LocalDate.of(2026, 11, 22))
        assertEquals(DateGroup.ThisWeek, group(sunday, firstDayOfWeek(Locale.US)))
        assertEquals(DateGroup.Month(2026, 11), group(sunday, firstDayOfWeek(Locale.UK)))
    }

    @Test
    fun onTheFirstDayOfTheWeekYesterdayIsStillYesterday() {
        val monday = LocalDate.of(2026, 11, 23)
        assertEquals(DateGroup.Yesterday, group(at(monday.minusDays(1)), on = monday))
        assertEquals(DateGroup.Month(2026, 11), group(at(monday.minusDays(2)), on = monday))
    }

    @Test
    fun yearBoundary() {
        val newYear = LocalDate.of(2027, 1, 1)
        assertEquals(DateGroup.Yesterday, group(at(LocalDate.of(2026, 12, 31)), on = newYear))
        // 1 January 2027 is a Friday: Monday 28 December is this week, the 27th is December.
        assertEquals(DateGroup.ThisWeek, group(at(LocalDate.of(2026, 12, 28)), on = newYear))
        assertEquals(DateGroup.Month(2026, 12), group(at(LocalDate.of(2026, 12, 27)), on = newYear))
    }

    @Test
    fun usesTheGivenTimeZone() {
        // 23:00 in London on the 24th is already the 25th in Tokyo.
        val late = at(today.minusDays(1), 23)
        assertEquals(DateGroup.Yesterday, group(late))
        assertEquals(DateGroup.Today, group(late, z = ZoneId.of("Asia/Tokyo")))
    }

    @Test
    fun daylightSavingDay() {
        // Clocks go back on 25 October 2026 in London: that day has 25 hours.
        val sunday = LocalDate.of(2026, 10, 25)
        val monday = LocalDate.of(2026, 10, 26)
        assertEquals(DateGroup.Yesterday, group(at(sunday, 0), on = monday))
        assertEquals(DateGroup.Yesterday, group(at(sunday, 23), on = monday))
        assertEquals(DateGroup.Today, group(at(monday, 0), on = monday))
    }

    @Test
    fun futureDatesAreToday() {
        assertEquals(DateGroup.Today, group(at(today.plusDays(3))))
        assertEquals(DateGroup.Today, group(at(LocalDate.of(9999, 12, 31))))
    }

    @Test
    fun undisplayableDatesNeverThrow() {
        assertEquals(DateGroup.NoDate, group(Long.MAX_VALUE))
        assertEquals(DateGroup.NoDate, group(Long.MIN_VALUE))
        assertEquals(DateGroup.NoDate, group(at(LocalDate.of(10000, 1, 1))))
        assertEquals(DateGroup.Month(1970, 1), group(0))
    }

    @Test
    fun headersGoBeforeEachRunWithUniqueKeys() {
        val dates =
            listOf(
                at(today, 9),
                at(today, 8),
                at(today.minusDays(1)),
                at(LocalDate.of(2026, 10, 3)),
                at(LocalDate.of(2025, 10, 3)),
                // Out of order: October 2026 again.
                at(LocalDate.of(2026, 10, 1)),
            )
        val entries = withDateHeaders(dates, { it }, today, zone, DayOfWeek.MONDAY)
        val shape =
            entries.map {
                when (it) {
                    is ListEntry.Header -> it.key
                    is ListEntry.Row -> "row"
                }
            }
        assertEquals(
            listOf(
                "h-today",
                "row",
                "row",
                "h-yesterday",
                "row",
                "h-2026-10",
                "row",
                "h-2025-10",
                "row",
                "h-2026-10-2",
                "row",
            ),
            shape,
        )
        assertEquals(0, withDateHeaders(emptyList<Long>(), { it }, today, zone, DayOfWeek.MONDAY).size)
    }
}
