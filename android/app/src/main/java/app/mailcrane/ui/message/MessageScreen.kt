package app.mailcrane.ui.message

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.res.Resources
import android.net.Uri
import android.os.Build
import android.text.format.Formatter
import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.mailcrane.R
import app.mailcrane.core.AttachmentInfo
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.InlinePart
import app.mailcrane.core.MessageAddress
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.ReplyKind
import app.mailcrane.core.TrackerInfo
import app.mailcrane.ui.compose.ReplyRequest
import app.mailcrane.ui.messagelist.errorText
import app.mailcrane.ui.theme.LocalDarkMessageBodies
import app.mailcrane.ui.theme.LocalDarkTheme
import java.time.DateTimeException
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** Message [messageId], opened from the list. */
@Composable
fun MessageScreen(
    messageId: Long,
    onBack: () -> Unit,
    folders: List<FolderSummary> = emptyList(),
    onReply: (ReplyRequest) -> Unit = {},
    viewModel: MessageViewModel =
        hiltViewModel<MessageViewModel, MessageViewModel.Factory>(
            key = "message-$messageId",
            creationCallback = { factory -> factory.create(messageId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    val back by rememberUpdatedState(onBack)
    LaunchedEffect(viewModel) {
        viewModel.effects.collect { effect ->
            when (effect) {
                MessageEffect.Close -> back()
                is MessageEffect.ActionRefused ->
                    snackbar.showSnackbar(
                        resources.getString(
                            when (effect.reason) {
                                ActionRefusal.NO_ARCHIVE_FOLDER -> R.string.action_no_archive_folder
                                ActionRefusal.CANNOT_SNOOZE -> R.string.snooze_no_message_id
                                ActionRefusal.FAILED -> R.string.action_not_applied
                            },
                        ),
                    )
            }
        }
    }
    var snoozing by rememberSaveable { mutableStateOf(false) }
    val askNotifications = rememberNotificationPermissionRequest()
    if (snoozing) {
        SnoozeSheet(
            onPick = {
                snoozing = false
                askNotifications()
                viewModel.snooze(it)
            },
            onDismiss = { snoozing = false },
        )
    }
    MessageContent(
        state = state,
        onBack = onBack,
        onUnsnooze = viewModel::unsnooze,
        snackbar = snackbar,
        folders = folders,
        onReply = { kind ->
            val ready = state as? MessageState.Ready ?: return@MessageContent
            onReply(ReplyRequest(ready.message.id, kind, quoteHeader(resources, ready.message, kind)))
        },
        actions =
            MessageActions(
                onArchive = viewModel::archive,
                onDelete = viewModel::delete,
                onMarkUnread = viewModel::markUnread,
                onToggleFlag = viewModel::toggleFlag,
                onMove = viewModel::moveTo,
                onSnooze = { snoozing = true },
            ),
        onRetry = viewModel::load,
        onLoadRemoteImages = viewModel::loadRemoteImages,
        inlinePart = viewModel::inlinePart,
        onOpenLink = { openLink(context, Uri.parse(viewModel.linkToOpen(it.toString()))) },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MessageContent(
    state: MessageState,
    onBack: () -> Unit,
    onRetry: () -> Unit,
    onLoadRemoteImages: () -> Unit,
    inlinePart: (String) -> InlinePart?,
    onOpenLink: (Uri) -> Unit,
    snackbar: SnackbarHostState = remember { SnackbarHostState() },
    folders: List<FolderSummary> = emptyList(),
    actions: MessageActions = MessageActions(),
    onReply: (ReplyKind) -> Unit = {},
    onUnsnooze: () -> Unit = {},
) {
    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = { if (state is MessageState.Ready) ReplyBar(onReply) },
        topBar = {
            TopAppBar(
                title = {},
                actions = {
                    if (state is MessageState.Ready) {
                        MessageToolbarActions(
                            flagged = state.message.isFlagged,
                            moveTargets = folders.filter { it.id != state.message.folderId },
                            actions = actions,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painter = painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.message_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when (state) {
                MessageState.Loading ->
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                is MessageState.Failed ->
                    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(stringResource(errorText(state.kind)))
                            TextButton(onClick = onRetry) { Text(stringResource(R.string.message_list_retry)) }
                        }
                    }
                is MessageState.Ready ->
                    Message(state, onLoadRemoteImages, inlinePart, onOpenLink, onUnsnooze)
            }
        }
    }
}

@Composable
private fun Message(
    state: MessageState.Ready,
    onLoadRemoteImages: () -> Unit,
    inlinePart: (String) -> InlinePart?,
    onOpenLink: (Uri) -> Unit,
    onUnsnooze: () -> Unit,
) {
    val message = state.message
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Headers(message)
        state.snooze?.let { SnoozedBanner(it, onUnsnooze) }
        val trackers = (message.body as? MessageBody.Html)?.trackers.orEmpty()
        if (trackers.isNotEmpty()) TrackersIndicator(trackers)
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        if (state.showRemoteBanner) RemoteImagesBanner(onLoadRemoteImages)
        when (val body = message.body) {
            is MessageBody.Plain -> PlainBody(body.text, onOpenLink)
            is MessageBody.Html -> {
                val dark = LocalDarkTheme.current
                val darken = dark && LocalDarkMessageBodies.current && darkeningSupported
                val webView = @Composable { m: Modifier ->
                    SafeWebView(
                        html = body.html,
                        allowRemote = state.remoteAllowed,
                        inlinePart = inlinePart,
                        onOpenLink = onOpenLink,
                        modifier = m.fillMaxWidth().testTag("message-html"),
                        darken = darken,
                    )
                }
                if (dark && !darken) {
                    // Most HTML mail assumes a white page: show it on a light "paper" card in dark mode.
                    Surface(
                        shape = RoundedCornerShape(12.dp),
                        color = androidx.compose.ui.graphics.Color.White,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                    ) { webView(Modifier.padding(4.dp)) }
                } else {
                    webView(Modifier)
                }
            }
            // The view model downloads the body before showing the message.
            MessageBody.NotDownloaded -> {}
        }
        if (message.attachments.isNotEmpty()) Attachments(message.attachments)
    }
}

@Composable
private fun Headers(message: MessageDetail) {
    val locale = LocalConfiguration.current.locales[0]
    val date = remember(message.date, locale) { formatFullDate(message.date, ZoneId.systemDefault(), locale) }
    Column(
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        SelectionContainer {
            Text(
                text = message.subject.ifBlank { stringResource(R.string.message_no_subject) },
                style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.semantics { heading() },
            )
        }
        val sender =
            label(message.from).ifBlank { stringResource(R.string.message_unknown_sender) }
        HeaderLine(stringResource(R.string.message_header_from), sender)
        if (message.to.isNotEmpty()) {
            HeaderLine(stringResource(R.string.message_header_to), message.to.joinToString(", ") { label(it) })
        }
        if (message.cc.isNotEmpty()) {
            HeaderLine(stringResource(R.string.message_header_cc), message.cc.joinToString(", ") { label(it) })
        }
        if (date.isNotEmpty()) {
            Text(
                text = date,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** "N trackers blocked" under the header; tapping lists them (M5-01). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TrackersIndicator(trackers: List<TrackerInfo>) {
    var open by rememberSaveable { mutableStateOf(false) }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable { open = true }
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .testTag("trackers-indicator"),
    ) {
        Icon(
            painter = painterResource(R.drawable.ic_mc_shield),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(18.dp),
        )
        Text(
            text = pluralStringResource(R.plurals.message_trackers_blocked, trackers.size, trackers.size),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
        )
    }
    if (open) {
        ModalBottomSheet(onDismissRequest = { open = false }) {
            TrackersSheet(trackers)
        }
    }
}

@Composable
internal fun TrackersSheet(trackers: List<TrackerInfo>) {
    Column(Modifier.navigationBarsPadding().padding(bottom = 16.dp).testTag("trackers-sheet")) {
        Text(
            text = stringResource(R.string.message_trackers_title),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp).semantics { heading() },
        )
        Text(
            text = stringResource(R.string.message_trackers_explain),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
        )
        trackers.forEach { tracker ->
            ListItem(
                headlineContent = {
                    Text(tracker.service ?: stringResource(R.string.message_trackers_unknown_service))
                },
                supportingContent = {
                    Text(tracker.domain.ifEmpty { stringResource(R.string.message_trackers_no_domain) })
                },
                leadingContent = {
                    Icon(painter = painterResource(R.drawable.ic_mc_shield), contentDescription = null)
                },
            )
        }
    }
}

@Composable
private fun HeaderLine(
    name: String,
    value: String,
) {
    SelectionContainer {
        Text(
            text =
                buildAnnotatedString {
                    pushStyle(SpanStyle(color = MaterialTheme.colorScheme.onSurfaceVariant))
                    append(name)
                    pop()
                    append(" ")
                    append(value)
                },
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}

@Composable
private fun RemoteImagesBanner(onLoadRemoteImages: () -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        shape = MaterialTheme.shapes.medium,
    ) {
        Row(
            modifier = Modifier.padding(start = 16.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = stringResource(R.string.message_remote_hidden),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = onLoadRemoteImages) { Text(stringResource(R.string.message_remote_load)) }
        }
    }
}

@Composable
private fun PlainBody(
    text: String,
    onOpenLink: (Uri) -> Unit,
) {
    val linkColor = MaterialTheme.colorScheme.primary
    val annotated =
        remember(text, linkColor) {
            linkify(text, TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))) {
                onOpenLink(Uri.parse(it))
            }
        }
    SelectionContainer {
        Text(
            text = annotated,
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.padding(16.dp).testTag("message-plain"),
        )
    }
}

@Composable
private fun Attachments(attachments: List<AttachmentInfo>) {
    val context = LocalContext.current
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            text = stringResource(R.string.message_attachments),
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.semantics { heading() },
        )
        attachments.forEach { a ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    painter = painterResource(R.drawable.ic_mc_attach),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(20.dp),
                )
                Column(Modifier.padding(start = 12.dp)) {
                    Text(
                        text = a.name.ifBlank { stringResource(R.string.message_attachment_unnamed) },
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Text(
                        text = "${a.mimeType} · ${formatSize(context, a.size)}",
                        style = MaterialTheme.typography.bodySmall.copy(fontSize = 12.sp),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

private fun label(a: MessageAddress): String =
    when {
        a.name.isNotBlank() && a.email.isNotBlank() -> "${a.name} <${a.email}>"
        else -> a.name.ifBlank { a.email }
    }

private fun formatSize(
    context: Context,
    size: ULong,
): String = Formatter.formatShortFileSize(context, size.coerceAtMost(Long.MAX_VALUE.toULong()).toLong())

/** Reply, Reply all and Forward, along the bottom of an open message. */
@Composable
private fun ReplyBar(onReply: (ReplyKind) -> Unit) {
    Surface(tonalElevation = 2.dp) {
        Row(
            Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 8.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            ReplyButton(R.drawable.ic_mc_reply, R.string.message_reply) { onReply(ReplyKind.REPLY) }
            ReplyButton(R.drawable.ic_mc_reply, R.string.message_reply_all) { onReply(ReplyKind.REPLY_ALL) }
            ReplyButton(R.drawable.ic_mc_forward, R.string.message_forward) { onReply(ReplyKind.FORWARD) }
        }
    }
}

@Composable
private fun ReplyButton(
    @DrawableRes icon: Int,
    @StringRes label: Int,
    onClick: () -> Unit,
) {
    TextButton(onClick = onClick) {
        Icon(painterResource(icon), contentDescription = null, modifier = Modifier.size(18.dp))
        Text(stringResource(label), modifier = Modifier.padding(start = 8.dp))
    }
}

/** The localised line above the quoted text, or the forward intro line. */
internal fun quoteHeader(
    resources: Resources,
    message: MessageDetail,
    kind: ReplyKind,
): String =
    when (kind) {
        ReplyKind.FORWARD -> resources.getString(R.string.forward_header)
        else -> {
            val sender = message.from.name.ifBlank { message.from.email }
            val date = formatFullDate(message.date, ZoneId.systemDefault(), Locale.getDefault())
            if (date.isEmpty()) {
                resources.getString(R.string.reply_quote_header_no_date, sender)
            } else {
                resources.getString(R.string.reply_quote_header, date, sender)
            }
        }
    }

/** Darkening HTML mail needs Android 13's algorithmic darkening. */
private val darkeningSupported: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

/** Opens a tapped link in the browser or mail app; never inside Mailcrane. */
internal fun openLink(
    context: Context,
    uri: Uri,
) {
    val intent =
        when (uri.scheme?.lowercase()) {
            "mailto" -> Intent(Intent.ACTION_SENDTO, uri)
            "https", "http" -> Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE)
            else -> return
        }
    try {
        context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (_: ActivityNotFoundException) {
        // No app can open it; nothing to do.
    }
}

/**
 * The full date and time for [epochSeconds] in [zone] and [locale]. Empty for a date outside the
 * years 1 to 9999 (the value comes from the message and may be anything).
 */
internal fun formatFullDate(
    epochSeconds: Long,
    zone: ZoneId,
    locale: Locale,
): String =
    try {
        val seconds = epochSeconds.coerceIn(Instant.MIN.epochSecond, Instant.MAX.epochSecond)
        val time = Instant.ofEpochSecond(seconds).atZone(zone)
        if (time.year !in 1..9999) {
            ""
        } else {
            DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT).withLocale(locale).format(time)
        }
    } catch (_: DateTimeException) {
        ""
    } catch (_: ArithmeticException) {
        ""
    }

// Web links (with a scheme or starting "www.") and email addresses. Deliberately strict, so plain
// words with dots aren't turned into links.
private val LINK =
    Regex("""(?i)\b(?:https?://|www\.)[^\s<>"]+|[A-Z0-9._%+-]+@[A-Z0-9-]+(?:\.[A-Z0-9-]+)*\.[A-Z]{2,}\b""")

/** Punctuation that usually ends a sentence rather than a link. */
private const val TRAILING = ".,;:!?)]}'\""

/**
 * [text] with web links and email addresses made into links. A tap calls [onClick] with the URL:
 * `https://…` for "www." links and `mailto:…` for addresses.
 */
internal fun linkify(
    text: String,
    styles: TextLinkStyles? = null,
    onClick: (String) -> Unit,
): AnnotatedString =
    buildAnnotatedString {
        append(text)
        for (match in LINK.findAll(text)) {
            var end = match.range.last + 1
            while (end > match.range.first && text[end - 1] in TRAILING) end--
            if (end <= match.range.first) continue
            val found = text.substring(match.range.first, end)
            val url =
                when {
                    found.contains("://") -> found
                    found.startsWith("www.", ignoreCase = true) -> "https://$found"
                    else -> "mailto:$found"
                }
            addLink(
                LinkAnnotation.Clickable(url, styles) { onClick(url) },
                match.range.first,
                end,
            )
        }
    }
