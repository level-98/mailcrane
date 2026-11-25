package app.mailcrane.widget

import android.content.Context
import android.content.Intent
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.LocalSize
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import app.mailcrane.MainActivity
import app.mailcrane.R
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.WidgetMessage
import app.mailcrane.core.WidgetSnapshot
import app.mailcrane.core.databasePath
import app.mailcrane.core.widgetSnapshot
import app.mailcrane.settings.AppSettings
import dagger.hilt.EntryPoint
import dagger.hilt.InstallIn
import dagger.hilt.android.EntryPointAccessors
import dagger.hilt.components.SingletonComponent
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext

/** Extras for opening a message from the widget (M4-11). */
const val EXTRA_MESSAGE_ID = "app.mailcrane.extra.MESSAGE_ID"
const val EXTRA_ACCOUNT_ID = "app.mailcrane.extra.ACCOUNT_ID"

@EntryPoint
@InstallIn(SingletonComponent::class)
interface WidgetEntryPoint {
    fun settings(): AppSettings
}

/** What the widget draws: the unread count, the messages, and whether content is hidden. */
data class WidgetContent(
    val unread: Long,
    val messages: List<WidgetMessage>,
    val hideContent: Boolean,
)

/** The widget's content from a snapshot; with [hideContent], messages are left out entirely. */
fun widgetContent(
    snapshot: WidgetSnapshot,
    hideContent: Boolean,
): WidgetContent =
    WidgetContent(
        unread = snapshot.unreadCount.toLong(),
        messages = if (hideContent) emptyList() else snapshot.messages,
        hideContent = hideContent,
    )

/**
 * The inbox widget (M4-11): every account's inbox together. Small, it shows the unread count; larger,
 * the latest messages (up to 5). Tapping a message opens it; tapping elsewhere opens the app. With "Hide
 * message content outside the app" on, only the count is shown.
 */
class InboxWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(SMALL, LARGE))

    override suspend fun provideGlance(
        context: Context,
        id: GlanceId,
    ) {
        val settings = EntryPointAccessors.fromApplication(context, WidgetEntryPoint::class.java).settings()
        val hide = settings.hideContentOutsideApp.first()
        val snapshot =
            withContext(Dispatchers.IO) {
                try {
                    widgetSnapshot(databasePath(context), MAX_MESSAGES.toUInt())
                } catch (_: SyncFailure) {
                    WidgetSnapshot(0u, emptyList())
                }
            }
        val content = widgetContent(snapshot, hide)
        provideContent { GlanceTheme { WidgetBody(context, content) } }
    }

    companion object {
        private const val MAX_MESSAGES = 5
        private val SMALL = DpSize(110.dp, 110.dp)
        private val LARGE = DpSize(250.dp, 180.dp)
    }
}

@Composable
private fun WidgetBody(
    context: Context,
    content: WidgetContent,
) {
    val openApp = actionStartActivity(Intent(context, MainActivity::class.java))
    val large = LocalSize.current.width >= 200.dp && content.messages.isNotEmpty()
    Column(
        modifier =
            GlanceModifier
                .fillMaxSize()
                .background(GlanceTheme.colors.widgetBackground)
                .cornerRadius(16.dp)
                .padding(12.dp)
                .clickable(openApp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = content.unread.toString(),
                style = TextStyle(fontSize = 28.sp, fontWeight = FontWeight.Bold, color = GlanceTheme.colors.primary),
            )
            Text(
                text = "  " + context.getString(R.string.widget_unread),
                style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant),
            )
        }
        if (large) {
            Spacer(GlanceModifier.height(8.dp))
            content.messages.forEach { m ->
                val open =
                    actionStartActivity(
                        Intent(context, MainActivity::class.java)
                            .setAction("app.mailcrane.action.OPEN_MESSAGE.${m.id}")
                            .putExtra(EXTRA_MESSAGE_ID, m.id)
                            .putExtra(EXTRA_ACCOUNT_ID, m.accountId),
                    )
                Column(GlanceModifier.fillMaxWidth().padding(vertical = 3.dp).clickable(open)) {
                    Text(
                        text = m.fromName.ifBlank { context.getString(R.string.message_unknown_sender) },
                        style =
                            TextStyle(
                                fontWeight = if (m.isSeen) FontWeight.Normal else FontWeight.Bold,
                                color = GlanceTheme.colors.onSurface,
                            ),
                        maxLines = 1,
                    )
                    Text(
                        text = m.subject.ifBlank { context.getString(R.string.message_no_subject) },
                        style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant),
                        maxLines = 1,
                    )
                }
            }
        }
    }
}

/** Registers [InboxWidget] with the launcher. */
class InboxWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = InboxWidget()
}
