package app.mailcrane.ui.message

import java.time.DayOfWeek
import java.time.LocalTime
import java.time.ZonedDateTime
import java.time.temporal.ChronoUnit
import java.time.temporal.TemporalAdjusters

/** The quick choices in the snooze picker (M4-06). */
enum class SnoozePreset {
    /** Three hours from now, rounded up to the next full hour. */
    LATER_TODAY,

    /** Tomorrow at 08:00. */
    TOMORROW,

    /** Saturday at 08:00: today if it's Saturday before 08:00, otherwise the coming Saturday. */
    THIS_WEEKEND,

    /** The coming Monday at 08:00 (never today). */
    NEXT_WEEK,
}

/** The hour the day-based presets wake at. */
private val MORNING: LocalTime = LocalTime.of(8, 0)

/**
 * When [preset] wakes, seen from [now] in its own time zone. Wall-clock times (08:00) stay 08:00 across
 * daylight-saving changes; "later today" adds real elapsed hours.
 */
fun snoozeTime(
    preset: SnoozePreset,
    now: ZonedDateTime,
): ZonedDateTime =
    when (preset) {
        SnoozePreset.LATER_TODAY -> {
            val later = now.plusHours(3)
            val hour = later.truncatedTo(ChronoUnit.HOURS)
            if (hour == later) hour else hour.plusHours(1)
        }
        SnoozePreset.TOMORROW -> morning(now.plusDays(1))
        SnoozePreset.THIS_WEEKEND -> {
            val today = morning(now)
            if (now.dayOfWeek == DayOfWeek.SATURDAY && now.isBefore(today)) {
                today
            } else {
                morning(now.with(TemporalAdjusters.next(DayOfWeek.SATURDAY)))
            }
        }
        SnoozePreset.NEXT_WEEK -> morning(now.with(TemporalAdjusters.next(DayOfWeek.MONDAY)))
    }

/** 08:00 on [day]'s date, in its zone. */
private fun morning(day: ZonedDateTime): ZonedDateTime = day.toLocalDate().atTime(MORNING).atZone(day.zone)
