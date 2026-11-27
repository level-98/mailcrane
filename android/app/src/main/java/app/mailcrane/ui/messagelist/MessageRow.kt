package app.mailcrane.ui.messagelist

import android.content.res.Resources
import android.icu.util.TimeZone
import android.text.format.DateFormat
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.mailcrane.R
import app.mailcrane.core.MessageSummary
import app.mailcrane.settings.MailboxDensity
import app.mailcrane.ui.components.SenderAvatar
import app.mailcrane.ui.theme.LocalMailboxDensity
import java.time.DateTimeException
import java.time.Instant
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/**
 * One message in the list. Unread rows are bold with a dot; flag and attachment have icons, so no
 * state is shown by colour alone. TalkBack reads the whole row as one item. A [selected] row (M4-14) is
 * tinted.
 *
 * Comfortable density (M4-24, the default) leads with the sender's picture: tapping it ([onAvatarClick])
 * selects the row, and a selected row shows a check mark there. Compact has no picture, puts subject and
 * preview on one line and uses tighter padding; while [selecting] a checkbox replaces the unread dot.
 * [avatarName] is the name the picture's initials come from (a conversation passes its newest sender).
 * A conversation's message [count] goes at the end of the second line, under the date, so it takes no
 * width from the first line and the date lines up with every other row's (#392). [dateStyle] picks the
 * list's short date or, inside a conversation, the date with its time (#399).
 */
@Composable
fun MessageRow(
    message: MessageSummary,
    now: ZonedDateTime,
    locale: Locale,
    modifier: Modifier = Modifier,
    selecting: Boolean = false,
    selected: Boolean = false,
    onAvatarClick: (() -> Unit)? = null,
    avatarName: String = message.fromName,
    count: Int? = null,
    dateStyle: RowDateStyle = RowDateStyle.LIST,
) {
    val comfortable = LocalMailboxDensity.current == MailboxDensity.COMFORTABLE
    val resources = LocalResources.current
    val sender = senderLabel(message, resources)
    val subject = message.subject.ifBlank { stringResource(R.string.message_no_subject) }
    val is24Hour = DateFormat.is24HourFormat(LocalContext.current)
    val date =
        remember(message.date, now.toLocalDate(), now.zone, locale, dateStyle, is24Hour) {
            when (dateStyle) {
                RowDateStyle.LIST -> formatListDate(message.date, now, locale)
                RowDateStyle.CONVERSATION -> formatConversationDate(message.date, now, locale, is24Hour)
            }
        }
    val countLabel = count?.let { pluralStringResource(R.plurals.thread_a11y_count, it, it) }
    val description = rowDescription(message, sender, subject, date, resources, selected, countLabel)
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
                    rowLeading =
                        when {
                            comfortable && selected -> RowLeading.CHECK_MARK
                            comfortable -> RowLeading.PICTURE
                            selecting -> RowLeading.CHECKBOX
                            else -> RowLeading.UNREAD_DOT
                        }
                }.then(if (selected) Modifier.background(colors.secondaryContainer) else Modifier)
                .padding(
                    start = 8.dp,
                    end = 16.dp,
                    top = if (comfortable) 10.dp else 6.dp,
                    bottom = if (comfortable) 10.dp else 6.dp,
                ),
        verticalAlignment = Alignment.Top,
    ) {
        if (comfortable) {
            // The unread dot stays beside the picture, so read state isn't only bold text.
            Box(
                modifier = Modifier.width(dotSize + 4.dp).padding(top = dotTop),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (unread) {
                    Box(Modifier.size(dotSize).background(colors.primary, CircleShape))
                }
            }
            Box(Modifier.padding(end = 12.dp)) {
                SenderAvatar(
                    name = avatarName,
                    address = message.fromAddr,
                    selected = selected,
                    onClick = onAvatarClick,
                )
                // In the Unified Inbox the account colour sits on the picture's edge (M4-01), offset outward
                // so it never covers the initials. Fixed in dp, like the circle, whatever the font scale.
                if (message.accountId != 0L) {
                    Box(
                        modifier =
                            Modifier
                                .align(Alignment.BottomEnd)
                                .offset(x = CORNER_DOT_OFFSET, y = CORNER_DOT_OFFSET)
                                .size(CORNER_DOT_RING)
                                .background(colors.surface, CircleShape),
                        contentAlignment = Alignment.Center,
                    ) {
                        app.mailcrane.ui.components
                            .AccountDot(message.accountColor.toInt(), size = CORNER_DOT)
                    }
                }
            }
        } else if (selecting) {
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
                // In the Unified Inbox, which account it came to (M4-01); on the picture when there is one.
                if (message.accountId != 0L && !comfortable) {
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
                val subjectLine =
                    if (comfortable || message.snippet.isBlank()) {
                        AnnotatedString(subject)
                    } else {
                        // Compact: subject and preview share one line.
                        buildAnnotatedString {
                            append(subject)
                            withStyle(SpanStyle(color = colors.onSurfaceVariant, fontWeight = FontWeight.Normal)) {
                                append("  ")
                                append(message.snippet)
                            }
                        }
                    }
                Text(
                    text = subjectLine,
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
                if (count != null) {
                    // TalkBack reads it from the row's description, so it has none of its own.
                    Text(
                        text = count.toString(),
                        modifier = Modifier.padding(start = 8.dp),
                        style = MaterialTheme.typography.labelLarge,
                        color = colors.onSurfaceVariant,
                        maxLines = 1,
                    )
                }
            }
            if (comfortable && message.snippet.isNotBlank()) {
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

/** The Unified Inbox account dot on a picture: its size, the surface ring around it, and how far it sits out. */
private val CORNER_DOT = 10.dp
private val CORNER_DOT_RING = 14.dp
private val CORNER_DOT_OFFSET = 4.dp

/** Which date a row shows: the list's short one, or a conversation's with the time (#399). */
enum class RowDateStyle { LIST, CONVERSATION }

/** What leads a message row (M4-24); for tests only, since TalkBack reads the row as one item. */
enum class RowLeading { PICTURE, CHECK_MARK, CHECKBOX, UNREAD_DOT }

internal val RowLeadingKey = SemanticsPropertyKey<RowLeading>("RowLeading")
internal var SemanticsPropertyReceiver.rowLeading by RowLeadingKey

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
    countLabel: String? = null,
): String =
    buildList {
        if (selected) add(resources.getString(R.string.message_a11y_selected))
        if (!message.isSeen) add(resources.getString(R.string.message_a11y_unread))
        if (message.isFlagged) add(resources.getString(R.string.message_a11y_flagged))
        add(resources.getString(R.string.message_a11y_from, sender))
        add(resources.getString(R.string.message_a11y_subject, subject))
        if (date.isNotEmpty()) add(resources.getString(R.string.message_a11y_date, date))
        if (message.hasAttachments) add(resources.getString(R.string.message_a11y_attachment))
        if (countLabel != null) add(resources.getString(R.string.message_a11y_count, countLabel))
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

/**
 * A message's date inside a conversation, with its time (#399): the time if it is today, the weekday
 * and time in the last week, otherwise the short date and time, in [locale] and [now]'s time zone.
 * [is24Hour] is the device's 12/24-hour setting; each pattern is the locale's own for an ICU skeleton
 * (`Hm` or `hm`, with `EEE` or `yMd`), so nothing is hard-coded. Empty for a date outside the years
 * 1 to 9999, as in [formatListDate].
 */
internal fun formatConversationDate(
    epochSeconds: Long,
    now: ZonedDateTime,
    locale: Locale,
    is24Hour: Boolean,
): String =
    try {
        val seconds = epochSeconds.coerceIn(Instant.MIN.epochSecond, Instant.MAX.epochSecond)
        val time = Instant.ofEpochSecond(seconds).atZone(now.zone)
        if (time.year !in 1..9999) return ""
        val day = time.toLocalDate()
        val today = now.toLocalDate()
        val clock = if (is24Hour) "Hm" else "hm"
        val skeleton =
            when {
                day == today -> clock
                day.isBefore(today) && day.isAfter(today.minusDays(7)) -> "EEE$clock"
                else -> "yMd$clock"
            }
        val format = android.icu.text.DateFormat.getInstanceForSkeleton(skeleton, locale)
        format.timeZone = TimeZone.getTimeZone(now.zone.id)
        format.format(java.util.Date(time.toInstant().toEpochMilli()))
    } catch (_: DateTimeException) {
        ""
    } catch (_: ArithmeticException) {
        ""
    }
