package app.mailcrane.settings

import app.mailcrane.R
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageSummary
import app.mailcrane.core.RuleActionInfo
import app.mailcrane.core.RuleConditionInfo
import app.mailcrane.core.RuleInfo
import app.mailcrane.core.RuleInput
import app.mailcrane.core.RuleLocation
import app.mailcrane.core.RuleRunsOn
import app.mailcrane.core.RulesCore
import app.mailcrane.core.RulesFailure
import app.mailcrane.core.ServerRulesNotice
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

private fun info(id: Long) =
    RuleInfo(
        id,
        RuleInput(
            accountId = null,
            name = "Rule $id",
            enabled = true,
            matchAll = true,
            stopProcessing = false,
            conditions = listOf(RuleConditionInfo.Subject("x")),
            actions = listOf(RuleActionInfo.Flag),
        ),
    )

private class FakeRules : RulesCore {
    var rules = listOf(info(1), info(2))
    var locations = emptyList<RuleLocation>()
    var notices = emptyList<ServerRulesNotice>()
    val switched = mutableListOf<Long>()

    override suspend fun listRules() = rules

    /** Thrown by [saveRule], as the core refuses a move into a folder that can't take messages (#465). */
    var saveFailure: RulesFailure? = null

    override suspend fun saveRule(
        id: Long?,
        rule: RuleInput,
    ): Long {
        saveFailure?.let { throw it }
        return 0L
    }

    override suspend fun deleteRule(id: Long) = Unit

    /** Thrown by [setRuleEnabled], as the core refuses a rule with no actions (#323). */
    var enableFailure: RulesFailure? = null

    override suspend fun setRuleEnabled(
        id: Long,
        enabled: Boolean,
    ) {
        enableFailure?.let { throw it }
    }

    override suspend fun moveRule(
        id: Long,
        index: Int,
    ) = Unit

    override suspend fun ruleLocations() = locations

    override suspend fun serverRulesNotices() = notices

    override suspend fun useServerRules(accountId: Long) {
        switched += accountId
        notices = notices.map { it.copy(switchRequested = true) }
    }
}

private class FakeMailbox : MailboxCore {
    override val events = MutableSharedFlow<CoreEvent>()
    val synced = mutableListOf<Long>()

    override suspend fun listMessages(
        folderId: Long,
        offset: Int,
        limit: Int,
    ) = emptyList<MessageSummary>()

    override suspend fun syncFolder(folderId: Long) {
        synced += folderId
    }

    override suspend fun loadOlder(
        folderId: Long,
        count: Int,
    ) = 0

    override suspend fun refreshFolders(accountId: Long): Long? = null

    override suspend fun listFolders(accountId: Long) =
        listOf(
            FolderSummary(id = 10, displayName = "INBOX", role = FolderRole.INBOX, depth = 0u, unreadCount = 0u),
        )

    override suspend fun swipeAction(
        messageId: Long,
        action: MessageAction,
        undoable: Boolean,
    ) = Unit

    override suspend fun undoAction(messageId: Long) = false

    override suspend fun searchMessages(
        accountId: Long,
        folderId: Long?,
        query: String,
        limit: Int,
    ) = emptyList<MessageSummary>()

    override suspend fun searchServer(
        folderId: Long,
        query: String,
    ) = 0

    override suspend fun searchAllAccounts(
        query: String,
        limit: Int,
    ) = emptyList<MessageSummary>()
}

@OptIn(ExperimentalCoroutinesApi::class)
class RulesViewModelTest {
    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    @Test
    fun aRemovedFolderShowsUpInTheOpenRulesScreen() =
        runTest {
            // Manage folders deletes a folder while the Rules screen stays open (#323).
            val core = FakeRules()
            val mailbox = FakeMailbox()
            val vm = RulesViewModel(core, mailbox, 7)
            assertEquals(
                null,
                vm.state.value.rules
                    .first()
                    .missingFolder,
            )
            core.rules = listOf(info(1).copy(missingFolder = "Receipts"), info(2))
            mailbox.events.emit(CoreEvent.FolderChanged(5))
            assertEquals(
                "Receipts",
                vm.state.value.rules
                    .first()
                    .missingFolder,
            )
            // And when the screen is shown again.
            core.rules = listOf(info(1), info(2).copy(missingFolder = "Old"))
            vm.refresh()
            assertEquals(
                "Old",
                vm.state.value.rules[1]
                    .missingFolder,
            )
        }

    @Test
    fun turningOnARuleWithNothingToDoIsExplained() =
        runTest {
            val core = FakeRules().apply { enableFailure = RulesFailure.Invalid() }
            val vm = RulesViewModel(core, FakeMailbox(), 7)
            vm.setEnabled(1, true)
            assertEquals(R.string.rules_enable_no_actions, vm.results.first().first)
        }

    @Test
    fun aRefusedSaveSaysWhyAndReopensTheEditor() =
        runTest {
            // #465 review: a move target the editor couldn't see as virtual (another account's Starred) is
            // refused by the core. The user's edits come back, with a message that says what to fix.
            val core = FakeRules().apply { saveFailure = RulesFailure.Invalid() }
            val vm = RulesViewModel(core, FakeMailbox(), 7)
            var reopened = false
            val draft =
                RuleDraft(
                    name = "Old",
                    conditions = listOf(ConditionDraft(ConditionKind.SUBJECT, "sale")),
                    actions = listOf(ActionDraft(ActionKind.MOVE, 99)),
                )
            assertEquals(true, vm.save(draft) { reopened = true })
            assertEquals(R.string.rules_target_refused, vm.results.first().first)
            assertEquals(true, reopened)
        }

    @Test
    fun withoutServerRulesNothingIsMarked() {
        val vm = RulesViewModel(FakeRules(), FakeMailbox(), 7)
        val state = vm.state.value
        assertEquals(2, state.rules.size)
        assertFalse(state.showsLocations)
        assertNull(state.notice)
    }

    @Test
    fun locationsAreShownPerRule() {
        val rules =
            FakeRules().apply {
                locations =
                    listOf(RuleLocation(1, RuleRunsOn.SERVER), RuleLocation(2, RuleRunsOn.DEVICE))
            }
        val state = RulesViewModel(rules, FakeMailbox(), 7).state.value
        assertTrue(state.showsLocations)
        assertEquals(RuleRunsOn.SERVER, state.locations[1])
        assertEquals(RuleRunsOn.DEVICE, state.locations[2])
    }

    @Test
    fun onlyThisAccountsNoticeShowsAndSwitchingIsExplicit() {
        val rules =
            FakeRules().apply {
                notices =
                    listOf(
                        ServerRulesNotice(3, "c@example.org", "Other", false),
                        ServerRulesNotice(7, "a@example.org", "Vacation", false),
                    )
            }
        val mailbox = FakeMailbox()
        val vm = RulesViewModel(rules, mailbox, 7)
        assertEquals(
            "Vacation",
            vm.state.value.notice
                ?.scriptName,
        )
        // Nothing switches by itself.
        assertTrue(rules.switched.isEmpty())
        vm.useServerRules()
        assertEquals(listOf(7L), rules.switched)
        assertEquals(listOf(10L), mailbox.synced)
        assertEquals(
            true,
            vm.state.value.notice
                ?.switchRequested,
        )
    }
}
