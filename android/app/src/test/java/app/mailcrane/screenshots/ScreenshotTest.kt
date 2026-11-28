package app.mailcrane.screenshots

import android.app.Application
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.AttachmentInfo
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MessageAddress
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.RuleActionInfo
import app.mailcrane.core.RuleConditionInfo
import app.mailcrane.core.RuleInfo
import app.mailcrane.core.RuleInput
import app.mailcrane.core.SaveSentSetting
import app.mailcrane.core.ThreadSummary
import app.mailcrane.settings.AccountColorContent
import app.mailcrane.settings.AccountColorState
import app.mailcrane.settings.RulesList
import app.mailcrane.settings.RulesState
import app.mailcrane.settings.SaveSentContent
import app.mailcrane.settings.SettingsContent
import app.mailcrane.settings.SignatureContent
import app.mailcrane.settings.SignatureState
import app.mailcrane.ui.compose.ComposeActions
import app.mailcrane.ui.compose.ComposeContent
import app.mailcrane.ui.compose.ComposeUiState
import app.mailcrane.ui.message.MessageContent
import app.mailcrane.ui.message.MessageState
import app.mailcrane.ui.messagelist.MessageListContent
import app.mailcrane.ui.messagelist.MessageListState
import app.mailcrane.ui.theme.MailcraneTheme
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.util.Locale
import java.util.TimeZone

/**
 * Screenshot tests of the main screens (#605), in light, dark and at the largest font scale (200 %), so a UI
 * change shows up as an image diff in CI instead of a manual screenshot pass. Synthetic data only.
 *
 * The goldens live in `src/test/screenshots/`. `./gradlew verifyRoborazziDebug` (run by `cargo xtask ci`)
 * fails on a diff and leaves the comparison images in `build/outputs/roborazzi/`; after an intended change,
 * `./gradlew recordRoborazziDebug` records them again (under `locks/heavy-job`).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class, qualifiers = "en-rGB-w411dp-h891dp-mdpi")
class ScreenshotTest {
    @Before
    fun fixedClockAndPlace() {
        // Dates render the same on every machine.
        TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
        Locale.setDefault(Locale.UK)
    }

    /** Captures [content] in light, dark and at 200 % text, as `<name>_light.png` and so on. */
    private fun capture(
        name: String,
        content: @Composable () -> Unit,
    ) {
        for ((variant, dark, fontScale) in VARIANTS) {
            captureRoboImage("src/test/screenshots/${name}_$variant.png") {
                val base = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(base.density, fontScale)) {
                    MailcraneTheme(darkTheme = dark) { Surface { content() } }
                }
            }
        }
    }

    @Test
    fun messageList() =
        capture("message_list") {
            MessageListContent(
                state =
                    MessageListState(
                        messages = (1L..6L).map(::message),
                        isLoading = false,
                        grouped = true,
                        threads = threads,
                    ),
                title = "Inbox",
                snackbar = SnackbarHostState(),
                onRefresh = {},
                onLoadMore = {},
                onOpenFolders = {},
                onCompose = {},
                onOpenSearch = {},
            )
        }

    @Test
    fun message() =
        capture("message") {
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

    @Test
    fun compose() =
        capture("compose") {
            ComposeContent(
                ComposeUiState(accounts = listOf(AccountSummary(1, "ann@example.org", "Ann")), fromAccountId = 1),
                ComposeActions(),
            )
        }

    @Test
    fun settings() = capture("settings") { SettingsContent(cleanLinks = true, onCleanLinksChange = {}, onBack = {}) }

    @Test
    fun accountSettings() =
        capture("account_settings") {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                SaveSentContent(current = SaveSentSetting.AUTOMATIC, onChange = {})
                SignatureContent(
                    state = SignatureState(loaded = true, text = "Ann\nexample.org"),
                    onTextChange = {},
                    onEnabledChange = {},
                    onSave = {},
                )
                AccountColorContent(state = AccountColorState(chosen = null, automatic = 3), onChange = {})
            }
        }

    @Test
    fun rules() =
        capture("rules") {
            RulesList(
                state =
                    RulesState(
                        rules = listOf(rule(1, "Receipts to a folder"), rule(2, "Flag the boss")),
                        loading = false,
                    ),
                snackbar = SnackbarHostState(),
                onBack = {},
                onAdd = {},
                onOpen = {},
                onEnabled = { _, _ -> },
                onMove = { _, _ -> },
            )
        }

    private fun message(id: Long) =
        MessageSummary(
            id = id,
            fromName = "Sender $id",
            fromAddr = "sender$id@example.org",
            subject = "Subject of message $id",
            snippet = "The first words of the message, which may run on for a while",
            date = DAY - id * 86_400,
            isSeen = id % 2 == 0L,
            isFlagged = id == 1L,
            hasAttachments = id == 2L,
        )

    private val threads =
        listOf(
            ThreadSummary(
                threadId = 10,
                subject = "Re: Release plans",
                participants = listOf("Dana Example", "Erik Example", "Test"),
                messageCount = 4u,
                unreadCount = 1u,
                lastDate = DAY,
                hasAttachments = false,
                isFlagged = false,
                newestMessageId = 1,
            ),
            ThreadSummary(
                threadId = 11,
                subject = "Re: Lunch on Friday?",
                participants = listOf("Carol Example", "Ann Example"),
                messageCount = 2u,
                unreadCount = 0u,
                lastDate = DAY - 86_400,
                hasAttachments = false,
                isFlagged = true,
                newestMessageId = 2,
            ),
        )

    private val detail =
        MessageDetail(
            id = 1,
            folderId = 7,
            from = MessageAddress("Ana Example", "ana@example.org"),
            to = listOf(MessageAddress("Bob", "bob@example.com")),
            cc = emptyList(),
            date = DAY,
            subject = "Plans for the week",
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
        )

    private fun rule(
        id: Long,
        name: String,
    ) = RuleInfo(
        id = id,
        rule =
            RuleInput(
                accountId = 1,
                name = name,
                enabled = id == 1L,
                matchAll = true,
                stopProcessing = false,
                conditions = listOf(RuleConditionInfo.Subject("receipt")),
                actions = listOf(RuleActionInfo.Flag),
            ),
        missingFolder = null,
    )

    private companion object {
        /** A fixed day in the past (Wednesday 15 January 2025, 10:00 UTC): no "today" or clock in the images. */
        const val DAY = 1_736_935_200L

        /** Name, dark theme, font scale. */
        val VARIANTS = listOf(Triple("light", false, 1f), Triple("dark", true, 1f), Triple("large", false, 2f))
    }
}
