package app.mailcrane.ui.message

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SelectableDates
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import app.mailcrane.R
import app.mailcrane.core.SnoozeInfo
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** The label of each preset. */
private fun SnoozePreset.label(): Int =
    when (this) {
        SnoozePreset.LATER_TODAY -> R.string.snooze_later_today
        SnoozePreset.TOMORROW -> R.string.snooze_tomorrow
        SnoozePreset.THIS_WEEKEND -> R.string.snooze_this_weekend
        SnoozePreset.NEXT_WEEK -> R.string.snooze_next_week
    }

/** A wake time as the user reads it: "Sat 28 Nov, 08:00" in their zone and locale. */
internal fun formatWake(
    time: ZonedDateTime,
    locale: Locale,
): String {
    val date = DateTimeFormatter.ofPattern("EEE d MMM", locale).format(time)
    val clock = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale).format(time)
    return "$date, $clock"
}

/**
 * The snooze picker (M4-06): the presets with their times, and a custom date and time. [onPick] gets
 * the wake time in Unix seconds.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SnoozeSheet(
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
    now: ZonedDateTime = remember { ZonedDateTime.now() },
) {
    var custom by rememberSaveable { mutableStateOf(false) }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        SnoozeChoices(now = now, onPick = onPick, onCustom = { custom = true })
    }
    if (custom) {
        CustomSnoozeDialogs(
            now = now,
            onPick = {
                custom = false
                onPick(it)
            },
            onDismiss = { custom = false },
        )
    }
}

/** The sheet's content, separate so it can be tested without a bottom sheet. */
@Composable
internal fun SnoozeChoices(
    now: ZonedDateTime,
    onPick: (Long) -> Unit,
    onCustom: () -> Unit,
) {
    val locale = LocalConfiguration.current.locales[0]
    Column(Modifier.navigationBarsPadding().padding(bottom = 16.dp).testTag("snooze-sheet")) {
        Text(
            text = stringResource(R.string.snooze_title),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp).semantics { heading() },
        )
        SnoozePreset.entries.forEach { preset ->
            val time = snoozeTime(preset, now)
            ListItem(
                headlineContent = { Text(stringResource(preset.label())) },
                trailingContent = { Text(formatWake(time, locale)) },
                modifier = Modifier.clickable { onPick(time.toEpochSecond()) },
            )
        }
        ListItem(
            headlineContent = { Text(stringResource(R.string.snooze_pick_date_time)) },
            modifier = Modifier.clickable(onClick = onCustom),
        )
    }
}

/** A date picker, then a time picker. Days before today can't be picked; a past time is refused. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CustomSnoozeDialogs(
    now: ZonedDateTime,
    onPick: (Long) -> Unit,
    onDismiss: () -> Unit,
) {
    val zone: ZoneId = now.zone
    val today = now.toLocalDate()
    var date by rememberSaveable { mutableStateOf<LocalDate?>(null) }
    val picked = date
    if (picked == null) {
        val dateState =
            rememberDatePickerState(
                // The date picker works in UTC midnights.
                initialSelectedDateMillis =
                    today
                        .plusDays(1)
                        .atStartOfDay(ZoneOffset.UTC)
                        .toInstant()
                        .toEpochMilli(),
                selectableDates =
                    object : SelectableDates {
                        override fun isSelectableDate(utcTimeMillis: Long): Boolean =
                            !Instant
                                .ofEpochMilli(utcTimeMillis)
                                .atZone(ZoneOffset.UTC)
                                .toLocalDate()
                                .isBefore(today)
                    },
            )
        DatePickerDialog(
            onDismissRequest = onDismiss,
            confirmButton = {
                TextButton(onClick = {
                    dateState.selectedDateMillis?.let {
                        date = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate()
                    }
                }) { Text(stringResource(R.string.snooze_next)) }
            },
            dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.snooze_cancel)) } },
        ) { DatePicker(state = dateState) }
    } else {
        val timeState = rememberTimePickerState(initialHour = 8, initialMinute = 0)
        var tooEarly by remember { mutableStateOf(false) }
        androidx.compose.ui.window.Dialog(onDismissRequest = onDismiss) {
            Surface(shape = MaterialTheme.shapes.extraLarge, tonalElevation = 6.dp) {
                Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    TimePicker(state = timeState)
                    if (tooEarly) {
                        Text(
                            text = stringResource(R.string.snooze_time_passed),
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        TextButton(onClick = onDismiss) { Text(stringResource(R.string.snooze_cancel)) }
                        TextButton(onClick = {
                            val wake = picked.atTime(LocalTime.of(timeState.hour, timeState.minute)).atZone(zone)
                            if (wake.isAfter(ZonedDateTime.now(zone))) onPick(wake.toEpochSecond()) else tooEarly = true
                        }) { Text(stringResource(R.string.snooze_confirm)) }
                    }
                }
            }
        }
    }
}

/**
 * Asks once for permission to post notifications (Android 13+), so a snoozed message can say when it
 * comes back. Snoozing works either way; without it the message just reappears.
 */
@Composable
internal fun rememberNotificationPermissionRequest(): () -> Unit {
    val context = LocalContext.current
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
    return {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}

/** Shown on a snoozed message: when it wakes, whether only this device knows, and Unsnooze. */
@Composable
internal fun SnoozedBanner(
    info: SnoozeInfo,
    onUnsnooze: () -> Unit,
) {
    val locale = LocalConfiguration.current.locales[0]
    val time = remember(info.wakeAt) { Instant.ofEpochSecond(info.wakeAt).atZone(ZoneId.systemDefault()) }
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        modifier = Modifier.fillMaxWidth().testTag("snoozed-banner"),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
        ) {
            Icon(
                painter = painterResource(R.drawable.ic_mc_snooze),
                contentDescription = null,
                modifier = Modifier.size(20.dp),
            )
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(
                    text = stringResource(R.string.snooze_until, formatWake(time, locale)),
                    style = MaterialTheme.typography.bodyMedium,
                )
                if (info.deviceOnly) {
                    Text(
                        text = stringResource(R.string.snooze_device_only),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                    )
                }
            }
            if (!info.waking) {
                TextButton(onClick = onUnsnooze) { Text(stringResource(R.string.snooze_unsnooze)) }
            }
        }
    }
}
