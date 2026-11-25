package app.mailcrane.ui.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import app.mailcrane.R
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.temporal.TemporalAdjusters
import java.util.Locale

/** A preset time to schedule a message for. */
enum class SchedulePreset { TOMORROW_MORNING, TOMORROW_AFTERNOON, MONDAY_MORNING }

/** When [preset] is, from [now], in Unix seconds. Monday means next Monday (a week ahead on a Monday). */
fun presetTime(
    preset: SchedulePreset,
    now: ZonedDateTime,
): Long {
    val date: LocalDate =
        when (preset) {
            SchedulePreset.TOMORROW_MORNING, SchedulePreset.TOMORROW_AFTERNOON -> now.toLocalDate().plusDays(1)
            SchedulePreset.MONDAY_MORNING -> now.toLocalDate().with(TemporalAdjusters.next(DayOfWeek.MONDAY))
        }
    val time = if (preset == SchedulePreset.TOMORROW_AFTERNOON) LocalTime.of(13, 0) else LocalTime.of(8, 0)
    return ZonedDateTime.of(date, time, now.zone).toEpochSecond()
}

/** A date and time as the user reads it, e.g. "26 Nov 2026, 08:00". */
fun formatSendAt(
    sendAt: Long,
    zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(),
): String =
    DateTimeFormatter
        .ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
        .withLocale(locale)
        .format(Instant.ofEpochSecond(sendAt).atZone(zone))

/** "Schedule send": presets, or a date and time of the user's choosing. */
@Composable
fun ScheduleButton(
    enabled: Boolean,
    onSchedule: (Long) -> Unit,
) {
    var open by rememberSaveable { mutableStateOf(false) }
    IconButton(onClick = { open = true }, enabled = enabled) {
        Icon(painterResource(R.drawable.ic_mc_snooze), contentDescription = stringResource(R.string.schedule_send))
    }
    if (open) {
        ScheduleDialog(
            onPick = {
                open = false
                onSchedule(it)
            },
            onDismiss = { open = false },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ScheduleDialog(
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
) {
    var step by rememberSaveable { mutableStateOf(0) }
    var pickedDate by rememberSaveable { mutableStateOf<Long?>(null) }
    val zone = ZoneId.systemDefault()
    when (step) {
        0 -> {
            val now = ZonedDateTime.now(zone)
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.schedule_send)) },
                text = {
                    LazyColumn {
                        items(SchedulePreset.entries) { preset ->
                            val at = presetTime(preset, now)
                            ListItem(
                                supportingContent = { Text(formatSendAt(at, zone)) },
                                modifier = Modifier.clickable(role = Role.Button) { onPick(at) },
                            ) {
                                Text(stringResource(presetLabel(preset)))
                            }
                        }
                        item {
                            ListItem(modifier = Modifier.clickable(role = Role.Button) { step = 1 }) {
                                Text(stringResource(R.string.schedule_pick))
                            }
                        }
                    }
                },
                confirmButton = {},
                dismissButton = { CancelButton(onDismiss) },
            )
        }
        1 -> {
            val dates = rememberDatePickerState()
            DatePickerDialog(
                onDismissRequest = onDismiss,
                confirmButton = {
                    TextButton(
                        onClick = {
                            pickedDate = dates.selectedDateMillis
                            step = 2
                        },
                        enabled = dates.selectedDateMillis != null,
                    ) { Text(stringResource(R.string.schedule_next)) }
                },
                dismissButton = { CancelButton(onDismiss) },
            ) {
                DatePicker(state = dates)
            }
        }
        else -> {
            val times = rememberTimePickerState(initialHour = 8, initialMinute = 0)
            AlertDialog(
                onDismissRequest = onDismiss,
                title = { Text(stringResource(R.string.schedule_pick)) },
                text = { TimePicker(state = times) },
                confirmButton = {
                    TextButton(onClick = {
                        // The date picker gives midnight UTC of the chosen day.
                        val millis = pickedDate ?: return@TextButton
                        val day = Instant.ofEpochMilli(millis).atZone(ZoneOffset.UTC).toLocalDate()
                        val at = ZonedDateTime.of(day, LocalTime.of(times.hour, times.minute), zone).toEpochSecond()
                        onPick(at)
                    }) { Text(stringResource(R.string.schedule_confirm)) }
                },
                dismissButton = { CancelButton(onDismiss) },
            )
        }
    }
}

@Composable
private fun CancelButton(onDismiss: () -> Unit) {
    TextButton(onClick = onDismiss) { Text(stringResource(R.string.message_move_cancel)) }
}

private fun presetLabel(preset: SchedulePreset): Int =
    when (preset) {
        SchedulePreset.TOMORROW_MORNING -> R.string.schedule_tomorrow_morning
        SchedulePreset.TOMORROW_AFTERNOON -> R.string.schedule_tomorrow_afternoon
        SchedulePreset.MONDAY_MORNING -> R.string.schedule_monday_morning
    }
