package app.mailcrane.a11y

import android.app.Application
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.Density
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.AttachmentInfo
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MessageAddress
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.MessageSummary
import app.mailcrane.settings.SettingsContent
import app.mailcrane.ui.compose.ComposeActions
import app.mailcrane.ui.compose.ComposeContent
import app.mailcrane.ui.compose.ComposeUiState
import app.mailcrane.ui.folders.ManageFoldersContent
import app.mailcrane.ui.folders.ManageFoldersState
import app.mailcrane.ui.message.MessageContent
import app.mailcrane.ui.message.MessageState
import app.mailcrane.ui.messagelist.MessageListContent
import app.mailcrane.ui.messagelist.MessageListState
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.ZonedDateTime

/**
 * The main screens pass [assertAccessible] at normal size and at the largest font scale, 200 %
 * (M6-01). A typical phone-sized window.
 */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "w411dp-h891dp")
class A11yScreensTest {
    @get:Rule
    val compose = createComposeRule()

    private val now = ZonedDateTime.now().toEpochSecond()

    /** The font scale the content is shown at; changed between the two checks. */
    private var fontScale by mutableFloatStateOf(1f)

    private fun show(content: @Composable () -> Unit) =
        compose.setContent {
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, fontScale)) {
                MailcraneTheme { content() }
            }
        }

    private fun message(id: Long) =
        MessageSummary(
            id = id,
            fromName = "Sender with a fairly long name $id",
            fromAddr = "sender$id@example.org",
            subject = "A subject long enough to need more than one line at large sizes $id",
            snippet = "The first words of the message, which may run on for a while",
            date = now - id * 3600,
            isSeen = id % 2 == 0L,
            isFlagged = id == 1L,
            hasAttachments = id == 2L,
        )

    private val detail =
        MessageDetail(
            id = 1,
            folderId = 7,
            from = MessageAddress("Ana Example", "ana@example.org"),
            to = listOf(MessageAddress("Bob", "bob@example.com")),
            cc = emptyList(),
            date = now,
            subject = "Plans for the week, and a subject long enough to wrap",
            isSeen = true,
            isFlagged = false,
            body = MessageBody.Plain("Hello Bob,\n\nSee you at lunch on Thursday.\n\nAna"),
            attachments = listOf(AttachmentInfo("agenda.pdf", "application/pdf", 20_000u, "2")),
        )

    private val folders =
        listOf(
            FolderSummary(1, "INBOX", FolderRole.INBOX, 0u, 3u),
            FolderSummary(2, "Sent", FolderRole.SENT, 0u, 0u),
            FolderSummary(3, "Projects", null, 0u, 0u, canChange = true),
            FolderSummary(4, "2026", null, 1u, 0u, canChange = true),
        )

    /** Shows [content] once, then checks it at 100 % and at 200 %. */
    private fun checkAtBothScales(
        screen: String,
        content: @Composable () -> Unit,
    ) {
        show(content)
        for (scale in listOf(1f, 2f)) {
            fontScale = scale
            compose.waitForIdle()
        }
    }

    @Test
    fun messageList() =
        checkAtBothScales("messageList") {
            run {
                MessageListContent(
                    state = MessageListState(messages = (1L..5L).map(::message), isLoading = false),
                    title = "Inbox",
                    snackbar = SnackbarHostState(),
                    onRefresh = {},
                    onLoadMore = {},
                    onOpenFolders = {},
                    onCompose = {},
                    onOpenSearch = {},
                )
            }
        }

    @Test
    fun message() =
        checkAtBothScales("message") {
            run {
                MessageContent(
                    state = MessageState.Ready(detail, remoteAllowed = false),
                    onBack = {},
                    onRetry = {},
                    onLoadRemoteImages = {},
                    inlinePart = { null },
                    onOpenLink = {},
                    folders = folders,
                )
            }
        }

    @Test
    fun compose() =
        checkAtBothScales("compose") {
            run {
                ComposeContent(
                    ComposeUiState(accounts = listOf(AccountSummary(1, "ann@example.org", "Ann")), fromAccountId = 1),
                    ComposeActions(),
                )
            }
        }

    @Test
    fun settings() =
        checkAtBothScales("settings") {
            run { SettingsContent(cleanLinks = true, onCleanLinksChange = {}, onBack = {}) }
        }

    @Test
    fun manageFolders() =
        checkAtBothScales("manageFolders") {
            run {
                ManageFoldersContent(
                    state = ManageFoldersState(loaded = true, folders = folders, canManage = true),
                    onBack = {},
                    onCreate = {},
                    onRename = {},
                    onMove = {},
                    onDelete = {},
                    onDismiss = {},
                    onConfirmCreate = {},
                    onConfirmRename = {},
                    onConfirmMove = {},
                    onConfirmDelete = {},
                )
            }
        }
}
