package app.mailcrane.ui.messagelist

import android.text.format.DateFormat
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.mailcrane.R
import java.time.DateTimeException
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.temporal.TemporalAdjusters
import java.time.temporal.WeekFields
import java.util.Locale

/** Which date header (M4-19) a row sits under. */
sealed interface DateGroup {
    /** Today, and anything dated later (a sender with a wrong clock). */
    data object Today : DateGroup

    data object Yesterday : DateGroup

    /** The earlier days of the current week, before yesterday. */
    data object ThisWeek : DateGroup

    data class Month(
        val year: Int,
        val month: Int,
    ) : DateGroup

    /** The message's date can't be shown (outside the years 1 to 9999). */
    data object NoDate : DateGroup
}

/**
 * The group for a row dated [epochSeconds], seen on [today] in [zone]. The week starts on
 * [firstDayOfWeek]. Never throws: the date comes from the message and may be any value.
 */
internal fun dateGroup(
    epochSeconds: Long,
    today: LocalDate,
    zone: ZoneId,
    firstDayOfWeek: DayOfWeek,
): DateGroup {
    val day =
        try {
            val seconds = epochSeconds.coerceIn(Instant.MIN.epochSecond, Instant.MAX.epochSecond)
            Instant.ofEpochSecond(seconds).atZone(zone).toLocalDate()
        } catch (_: DateTimeException) {
            return DateGroup.NoDate
        } catch (_: ArithmeticException) {
            return DateGroup.NoDate
        }
    if (day.year !in 1..9999) return DateGroup.NoDate
    val yesterday = today.minusDays(1)
    val weekStart = today.with(TemporalAdjusters.previousOrSame(firstDayOfWeek))
    return when {
        !day.isBefore(today) -> DateGroup.Today
        day == yesterday -> DateGroup.Yesterday
        !day.isBefore(weekStart) -> DateGroup.ThisWeek
        else -> DateGroup.Month(day.year, day.monthValue)
    }
}

/** One entry in the list: a date header, or a row. */
sealed interface ListEntry<out T> {
    data class Header(
        val group: DateGroup,
        val key: String,
    ) : ListEntry<Nothing>

    data class Row<T>(
        val item: T,
    ) : ListEntry<T>
}

/**
 * [rows] (newest first) with a [ListEntry.Header] before each run of rows in the same group.
 * Header keys are unique even if a group comes back (the list should be sorted, but the
 * dates come from messages).
 */
internal fun <T> withDateHeaders(
    rows: List<T>,
    dateOf: (T) -> Long,
    today: LocalDate,
    zone: ZoneId,
    firstDayOfWeek: DayOfWeek,
): List<ListEntry<T>> {
    val out = ArrayList<ListEntry<T>>(rows.size + 8)
    val seen = HashMap<DateGroup, Int>()
    var current: DateGroup? = null
    for (row in rows) {
        val group = dateGroup(dateOf(row), today, zone, firstDayOfWeek)
        if (group != current) {
            val times = (seen[group] ?: 0) + 1
            seen[group] = times
            val base = "h-${groupKey(group)}"
            out += ListEntry.Header(group, if (times == 1) base else "$base-$times")
            current = group
        }
        out += ListEntry.Row(row)
    }
    return out
}

private fun groupKey(group: DateGroup): String =
    when (group) {
        DateGroup.Today -> "today"
        DateGroup.Yesterday -> "yesterday"
        DateGroup.ThisWeek -> "week"
        DateGroup.NoDate -> "none"
        is DateGroup.Month -> "${group.year}-${group.month}"
    }

/** The locale's first day of the week, e.g. Monday in the UK and Sunday in the US. */
internal fun firstDayOfWeek(locale: Locale): DayOfWeek = WeekFields.of(locale).firstDayOfWeek

/**
 * A month header in [locale]'s own form: "November" in the current year, "November 2025" for
 * earlier ones ("2025年11月" in Japanese).
 */
internal fun monthLabel(
    month: DateGroup.Month,
    currentYear: Int,
    locale: Locale,
): String {
    val skeleton = if (month.year == currentYear) "MMMM" else "MMMMyyyy"
    val date = LocalDate.of(month.year, month.month, 1)
    val localized =
        try {
            DateTimeFormatter.ofPattern(DateFormat.getBestDateTimePattern(locale, skeleton), locale).format(date)
        } catch (_: IllegalArgumentException) {
            null
        } catch (_: DateTimeException) {
            null
        }
    // ICU may use a pattern letter java.time doesn't know; fall back to a plain form.
    return localized
        ?: DateTimeFormatter.ofPattern(if (month.year == currentYear) "LLLL" else "LLLL yyyy", locale).format(date)
}

/** A date header: a TalkBack heading, so the headings gesture jumps between groups. */
@Composable
internal fun DateHeader(
    group: DateGroup,
    today: LocalDate,
    locale: Locale,
    modifier: Modifier = Modifier,
) {
    val label =
        when (group) {
            DateGroup.Today -> stringResource(R.string.message_list_header_today)
            DateGroup.Yesterday -> stringResource(R.string.message_list_header_yesterday)
            DateGroup.ThisWeek -> stringResource(R.string.message_list_header_this_week)
            DateGroup.NoDate -> stringResource(R.string.message_list_header_no_date)
            is DateGroup.Month -> monthLabel(group, today.year, locale)
        }
    Surface(color = MaterialTheme.colorScheme.surface, modifier = modifier.fillMaxWidth()) {
        Text(
            text = label,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier =
                Modifier
                    .padding(start = 24.dp, end = 16.dp, top = 16.dp, bottom = 6.dp)
                    .semantics { heading() }
                    .testTag("date-header"),
        )
    }
}
