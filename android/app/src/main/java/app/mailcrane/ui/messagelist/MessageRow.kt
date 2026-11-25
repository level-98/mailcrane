package app.mailcrane.ui.messagelist

import android.content.res.Resources
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.mailcrane.R
import app.mailcrane.core.MessageSummary
import java.time.DateTimeException
import java.time.Instant
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/**
 * One message in the list. Unread rows are bold with a dot; flag and attachment have icons, so no
 * state is shown by colour alone. TalkBack reads the whole row as one item. While [selecting] (M4-14) a
 * checkbox replaces the unread dot and a [selected] row is tinted.
 */
@Composable
fun MessageRow(
    message: MessageSummary,
    now: ZonedDateTime,
    locale: Locale,
    modifier: Modifier = Modifier,
    selecting: Boolean = false,
    selected: Boolean = false,
) {
    val resources = LocalResources.current
    val sender = senderLabel(message, resources)
    val subject = message.subject.ifBlank { stringResource(R.string.message_no_subject) }
    val date = remember(message.date, now.toLocalDate(), now.zone, locale) { formatListDate(message.date, now, locale) }
    val description = rowDescription(message, sender, subject, date, resources, selected)
    val unread = !message.isSeen
    val weight = if (unread) FontWeight.Bold else FontWeight.Normal
    val colors = MaterialTheme.colorScheme
    // Markers grow with the text, so they stay readable and aligned at large font sizes.
    val density = LocalDensity.current
    val iconSize = with(density) { 16.sp.toDp() }
    val dotSize = with(density) { 8.sp.toDp() }
    // Centres the dot on the sender line (titleSmall: 20sp line height).
    val dotTop = with(density) { 6.sp.toDp() }

    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .clearAndSetSemantics {
                    contentDescription = description
                    if (selecting) this.selected = selected
                }.then(if (selected) Modifier.background(colors.secondaryContainer) else Modifier)
                .padding(start = 8.dp, end = 16.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.Top,
    ) {
        if (selecting) {
            // Not clickable itself: the row handles taps, so the target stays the whole row.
            Box(Modifier.padding(end = 8.dp), contentAlignment = Alignment.Center) {
                Checkbox(checked = selected, onCheckedChange = null)
            }
        } else {
            Box(
                modifier = Modifier.width(16.dp + dotSize / 2).padding(top = dotTop),
                contentAlignment = Alignment.Center,
            ) {
                if (unread) {
                    Box(Modifier.size(dotSize).background(colors.primary, CircleShape))
                }
            }
        }
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // In the Unified Inbox, which account it came to (M4-01).
                if (message.accountId != 0L) {
                    Box(Modifier.padding(end = 6.dp)) {
                        app.mailcrane.ui.components
                            .AccountDot(message.accountColor.toInt(), size = dotSize)
                    }
                }
                Text(
                    text = sender,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = weight,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = date,
                    modifier = Modifier.padding(start = 8.dp),
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = weight,
                    color = if (unread) colors.primary else colors.onSurfaceVariant,
                    maxLines = 1,
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = subject,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = weight,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (message.hasAttachments) {
                    Icon(
                        painter = painterResource(R.drawable.ic_mc_attach),
                        contentDescription = null,
                        modifier = Modifier.padding(start = 6.dp).size(iconSize),
                        tint = colors.onSurfaceVariant,
                    )
                }
                if (message.isFlagged) {
                    Icon(
                        painter = painterResource(R.drawable.ic_mc_star),
                        contentDescription = null,
                        modifier = Modifier.padding(start = 6.dp).size(iconSize),
                        tint = colors.primary,
                    )
                }
            }
            if (message.snippet.isNotBlank()) {
                Text(
                    text = message.snippet,
                    style = MaterialTheme.typography.bodySmall,
                    color = colors.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/** The sender's name, else their address, else "Unknown sender". */
internal fun senderLabel(
    message: MessageSummary,
    resources: Resources,
): String =
    message.fromName.ifBlank { message.fromAddr }.ifBlank {
        resources.getString(R.string.message_unknown_sender)
    }

/** What TalkBack reads, e.g. "Unread. From Ana. Subject: Lunch. 10:42. Has attachment.", led by "Selected." */
internal fun rowDescription(
    message: MessageSummary,
    sender: String,
    subject: String,
    date: String,
    resources: Resources,
    selected: Boolean = false,
): String =
    buildList {
        if (selected) add(resources.getString(R.string.message_a11y_selected))
        if (!message.isSeen) add(resources.getString(R.string.message_a11y_unread))
        if (message.isFlagged) add(resources.getString(R.string.message_a11y_flagged))
        add(resources.getString(R.string.message_a11y_from, sender))
        add(resources.getString(R.string.message_a11y_subject, subject))
        if (date.isNotEmpty()) add(resources.getString(R.string.message_a11y_date, date))
        if (message.hasAttachments) add(resources.getString(R.string.message_a11y_attachment))
    }.joinToString(" ")

/**
 * The list date for [epochSeconds]: the time if it is today, the weekday if it is in the last
 * week, otherwise a short date, in [locale] and [now]'s time zone. Empty for a date outside the
 * years 1 to 9999 (the value comes from the message and may be anything).
 */
internal fun formatListDate(
    epochSeconds: Long,
    now: ZonedDateTime,
    locale: Locale,
): String =
    try {
        val seconds = epochSeconds.coerceIn(Instant.MIN.epochSecond, Instant.MAX.epochSecond)
        val time = Instant.ofEpochSecond(seconds).atZone(now.zone)
        if (time.year !in 1..9999) return ""
        val day = time.toLocalDate()
        val today = now.toLocalDate()
        val formatter =
            when {
                day == today -> DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)
                day.isBefore(today) && day.isAfter(today.minusDays(7)) -> DateTimeFormatter.ofPattern("EEE")
                else -> DateTimeFormatter.ofLocalizedDate(FormatStyle.SHORT)
            }
        formatter.withLocale(locale).format(time)
    } catch (_: DateTimeException) {
        ""
    } catch (_: ArithmeticException) {
        ""
    }
