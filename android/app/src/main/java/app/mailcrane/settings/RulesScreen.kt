package app.mailcrane.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.RuleActionInfo
import app.mailcrane.core.RuleConditionInfo
import app.mailcrane.core.RuleInfo
import app.mailcrane.core.RuleInput
import app.mailcrane.core.RuleRunsOn
import app.mailcrane.core.RulesCore
import app.mailcrane.core.RulesFailure
import app.mailcrane.core.ServerRulesNotice
import app.mailcrane.core.SyncFailure
import app.mailcrane.ui.folders.folderName
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.io.Serializable

/** The kinds of condition the editor offers, in menu order. */
enum class ConditionKind(
    val label: Int,
    val needsText: Boolean,
) {
    FROM(R.string.rule_cond_from, true),
    TO_CC(R.string.rule_cond_to_cc, true),
    SUBJECT(R.string.rule_cond_subject, true),
    HAS_ATTACHMENT(R.string.rule_cond_attachment, false),
    LIST_ID(R.string.rule_cond_list_id, true),
    OLDER_THAN(R.string.rule_cond_older, true),
}

/** The kinds of action the editor offers, in menu order. */
enum class ActionKind(
    val label: Int,
) {
    MOVE(R.string.rule_act_move),
    MARK_READ(R.string.rule_act_read),
    FLAG(R.string.rule_act_flag),
    ARCHIVE(R.string.rule_act_archive),
    DELETE(R.string.rule_act_delete),
}

data class ConditionDraft(
    val kind: ConditionKind = ConditionKind.FROM,
    val text: String = "",
) : Serializable

data class ActionDraft(
    val kind: ActionKind = ActionKind.MOVE,
    val folderId: Long? = null,
) : Serializable

/** A rule being edited. [id] is null for a new one. */
data class RuleDraft(
    val id: Long? = null,
    val name: String = "",
    val enabled: Boolean = true,
    val matchAll: Boolean = true,
    val stopProcessing: Boolean = false,
    /** Only for this account (`false`: every account). */
    val thisAccountOnly: Boolean = true,
    val conditions: List<ConditionDraft> = listOf(ConditionDraft()),
    val actions: List<ActionDraft> = listOf(ActionDraft()),
) : Serializable {
    /** What the core needs, or null if something is missing. */
    fun toInput(accountId: Long): RuleInput? {
        if (name.isBlank()) return null
        val conds =
            conditions.map { c ->
                val t = c.text.trim()
                if (c.kind.needsText && t.isEmpty()) return null
                when (c.kind) {
                    ConditionKind.FROM -> RuleConditionInfo.From(t)
                    ConditionKind.TO_CC -> RuleConditionInfo.ToOrCc(t)
                    ConditionKind.SUBJECT -> RuleConditionInfo.Subject(t)
                    ConditionKind.HAS_ATTACHMENT -> RuleConditionInfo.HasAttachment
                    ConditionKind.LIST_ID -> RuleConditionInfo.ListId(t)
                    ConditionKind.OLDER_THAN -> RuleConditionInfo.OlderThanDays(t.toUIntOrNull() ?: return null)
                }
            }
        val acts =
            actions.map { a ->
                when (a.kind) {
                    ActionKind.MOVE -> RuleActionInfo.Move(a.folderId ?: return null)
                    ActionKind.MARK_READ -> RuleActionInfo.MarkRead
                    ActionKind.FLAG -> RuleActionInfo.Flag
                    ActionKind.ARCHIVE -> RuleActionInfo.Archive
                    ActionKind.DELETE -> RuleActionInfo.Delete
                }
            }
        if (conds.isEmpty() || acts.isEmpty()) return null
        return RuleInput(
            accountId = if (thisAccountOnly) accountId else null,
            name = name.trim(),
            enabled = enabled,
            matchAll = matchAll,
            stopProcessing = stopProcessing,
            conditions = conds,
            actions = acts,
        )
    }

    companion object {
        /** "Create rule from this message": from this sender, move to … (the user picks). */
        fun fromSender(address: String): RuleDraft =
            RuleDraft(name = address, conditions = listOf(ConditionDraft(ConditionKind.FROM, address)))

        fun of(rule: RuleInfo): RuleDraft =
            RuleDraft(
                id = rule.id,
                name = rule.rule.name,
                enabled = rule.rule.enabled,
                matchAll = rule.rule.matchAll,
                stopProcessing = rule.rule.stopProcessing,
                thisAccountOnly = rule.rule.accountId != null,
                conditions =
                    rule.rule.conditions.map {
                        when (it) {
                            is RuleConditionInfo.From -> ConditionDraft(ConditionKind.FROM, it.text)
                            is RuleConditionInfo.ToOrCc -> ConditionDraft(ConditionKind.TO_CC, it.text)
                            is RuleConditionInfo.Subject -> ConditionDraft(ConditionKind.SUBJECT, it.text)
                            RuleConditionInfo.HasAttachment -> ConditionDraft(ConditionKind.HAS_ATTACHMENT)
                            is RuleConditionInfo.ListId -> ConditionDraft(ConditionKind.LIST_ID, it.id)
                            is RuleConditionInfo.OlderThanDays ->
                                ConditionDraft(ConditionKind.OLDER_THAN, it.days.toString())
                        }
                    },
                actions =
                    rule.rule.actions.map {
                        when (it) {
                            is RuleActionInfo.Move -> ActionDraft(ActionKind.MOVE, it.folderId)
                            RuleActionInfo.MarkRead -> ActionDraft(ActionKind.MARK_READ)
                            RuleActionInfo.Flag -> ActionDraft(ActionKind.FLAG)
                            RuleActionInfo.Archive -> ActionDraft(ActionKind.ARCHIVE)
                            RuleActionInfo.Delete -> ActionDraft(ActionKind.DELETE)
                        }
                    },
            )
    }
}

data class RulesState(
    val rules: List<RuleInfo> = emptyList(),
    val folders: List<FolderSummary> = emptyList(),
    val loading: Boolean = true,
    /** Where each rule runs, by rule id (M4-23). Missing: unmarked. */
    val locations: Map<Long, RuleRunsOn> = emptyMap(),
    /** This account's server runs another filter script, so its rules stay here. */
    val notice: ServerRulesNotice? = null,
) {
    /** Whether any rule shows where it runs. */
    val showsLocations: Boolean get() = locations.values.any { it != RuleRunsOn.UNMARKED }
}

@HiltViewModel(assistedFactory = RulesViewModel.Factory::class)
class RulesViewModel
    @AssistedInject
    constructor(
        private val core: RulesCore,
        private val mailbox: MailboxCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): RulesViewModel
        }

        private val _state = MutableStateFlow(RulesState())
        val state: StateFlow<RulesState> = _state.asStateFlow()

        /** Results to show in a snackbar: a string resource and a count (or -1). */
        private val _results = Channel<Pair<Int, Int>>(Channel.BUFFERED)
        val results: Flow<Pair<Int, Int>> = _results.receiveAsFlow()

        init {
            viewModelScope.launch {
                val folders =
                    try {
                        mailbox.listFolders(accountId)
                    } catch (_: SyncFailure) {
                        emptyList()
                    }
                _state.update { it.copy(folders = folders) }
                reload()
            }
        }

        private suspend fun reload() {
            val rules =
                try {
                    core.listRules()
                } catch (_: RulesFailure) {
                    _results.send(R.string.rules_failed to -1)
                    _state.value.rules
                }
            // Where rules run is extra information: without it, nothing is marked.
            val locations =
                try {
                    core.ruleLocations().associate { it.ruleId to it.runsOn }
                } catch (_: RulesFailure) {
                    emptyMap()
                }
            val notice =
                try {
                    core.serverRulesNotices().firstOrNull { it.accountId == accountId }
                } catch (_: RulesFailure) {
                    null
                }
            // Only rules this account sees: its own and the ones for every account.
            _state.update { s ->
                s.copy(
                    rules =
                        rules.filter {
                            it.rule.accountId == null ||
                                it.rule.accountId == accountId
                        },
                    loading = false,
                    locations = locations,
                    notice = notice,
                )
            }
        }

        private fun act(block: suspend () -> Unit) {
            viewModelScope.launch {
                try {
                    block()
                } catch (_: RulesFailure) {
                    _results.send(R.string.rules_failed to -1)
                }
                reload()
            }
        }

        /** Saves [draft]; returns false (nothing saved) when something is missing. */
        fun save(draft: RuleDraft): Boolean {
            val input = draft.toInput(accountId) ?: return false
            act { core.saveRule(draft.id, input) }
            return true
        }

        fun delete(id: Long) = act { core.deleteRule(id) }

        fun setEnabled(
            id: Long,
            enabled: Boolean,
        ) = act { core.setRuleEnabled(id, enabled) }

        /** Moves a rule one place up (-1) or down (+1). */
        fun move(
            id: Long,
            by: Int,
        ) {
            val all = _state.value.rules
            val i = all.indexOfFirst { it.id == id }
            if (i < 0) return
            val target = all.getOrNull(i + by) ?: return
            // Positions are across every rule, including other accounts'.
            act {
                val everything = core.listRules()
                core.moveRule(id, everything.indexOfFirst { it.id == target.id })
            }
        }

        /**
         * The user confirmed running the rules on the server instead of its other filter script.
         * A sync of the inbox right after carries it out.
         */
        fun useServerRules() {
            viewModelScope.launch {
                try {
                    core.useServerRules(accountId)
                } catch (_: RulesFailure) {
                    _results.send(R.string.rules_failed to -1)
                    return@launch
                }
                reload()
                val inbox = _state.value.folders.firstOrNull { it.role == FolderRole.INBOX } ?: return@launch
                try {
                    mailbox.syncFolder(inbox.id)
                } catch (_: SyncFailure) {
                    // The next sync tries again.
                }
                reload()
            }
        }

        /** Runs a saved rule on this account's inbox now. */
        fun runOnInbox(id: Long) {
            val inbox = _state.value.folders.firstOrNull { it.role == FolderRole.INBOX } ?: return
            viewModelScope.launch {
                try {
                    val changed = mailbox.runRuleOnExisting(id, inbox.id, accountId)
                    _results.send(R.plurals.rules_ran to changed)
                } catch (_: RulesFailure) {
                    _results.send(R.string.rules_failed to -1)
                }
            }
        }
    }

/** The rules list (M4-09); [startWith] opens the editor at once (e.g. "Create rule from this message"). */
@Composable
fun RulesScreen(
    accountId: Long,
    onBack: () -> Unit,
    startWith: RuleDraft? = null,
    viewModel: RulesViewModel =
        hiltViewModel<RulesViewModel, RulesViewModel.Factory>(
            key = "rules-$accountId",
            creationCallback = { factory -> factory.create(accountId) },
        ),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var editing by rememberSaveable { mutableStateOf(startWith) }
    val snackbar = remember { SnackbarHostState() }
    val resources = LocalResources.current
    LaunchedEffect(viewModel) {
        viewModel.results.collect { (res, n) ->
            val text =
                if (n >= 0) resources.getQuantityString(res, n, n) else resources.getString(res)
            snackbar.showSnackbar(text)
        }
    }
    val draft = editing
    if (draft != null) {
        RuleEditor(
            draft = draft,
            folders = state.folders,
            onChange = { editing = it },
            onSave = { if (viewModel.save(it)) editing = null },
            onCancel = { editing = null },
            onDelete =
                draft.id?.let { id ->
                    {
                        viewModel.delete(id)
                        editing = null
                    }
                },
            onRunNow = draft.id?.let { id -> { viewModel.runOnInbox(id) } },
        )
        return
    }
    RulesList(
        state = state,
        snackbar = snackbar,
        onBack = onBack,
        onAdd = { editing = RuleDraft() },
        onOpen = { editing = RuleDraft.of(it) },
        onEnabled = viewModel::setEnabled,
        onMove = viewModel::move,
        onUseServerRules = viewModel::useServerRules,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RulesList(
    state: RulesState,
    snackbar: SnackbarHostState,
    onBack: () -> Unit,
    onAdd: () -> Unit,
    onOpen: (RuleInfo) -> Unit,
    onEnabled: (Long, Boolean) -> Unit,
    onMove: (Long, Int) -> Unit,
    onUseServerRules: () -> Unit = {},
) {
    var confirmSwitch by rememberSaveable { mutableStateOf(false) }
    val notice = state.notice
    if (confirmSwitch && notice != null) {
        AlertDialog(
            onDismissRequest = { confirmSwitch = false },
            title = { Text(stringResource(R.string.rules_switch_title)) },
            text = { Text(stringResource(R.string.rules_switch_body, notice.scriptName, notice.accountEmail)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmSwitch = false
                    onUseServerRules()
                }) { Text(stringResource(R.string.rules_switch_confirm)) }
            },
            dismissButton = {
                TextButton(onClick = { confirmSwitch = false }) { Text(stringResource(R.string.rules_cancel)) }
            },
        )
    }
    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.rules_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.settings_back),
                        )
                    }
                },
                actions = { TextButton(onClick = onAdd) { Text(stringResource(R.string.rules_add)) } },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())) {
            Text(
                text =
                    stringResource(
                        if (state.showsLocations) R.string.rules_explain_server else R.string.rules_explain,
                    ),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(16.dp),
            )
            if (notice != null) {
                ServerRulesNoticeRow(notice = notice, onSwitch = { confirmSwitch = true })
            }
            if (!state.loading && state.rules.isEmpty()) {
                Text(stringResource(R.string.rules_empty), modifier = Modifier.padding(horizontal = 16.dp))
            }
            state.rules.forEachIndexed { index, rule ->
                ListItem(
                    headlineContent = { Text(rule.rule.name) },
                    supportingContent = {
                        val n = rule.rule.conditions.size
                        Column {
                            Text(pluralStringResource(R.plurals.rules_summary, n, n))
                            when (state.locations[rule.id]) {
                                RuleRunsOn.SERVER -> Text(stringResource(R.string.rules_on_server))
                                RuleRunsOn.DEVICE -> Text(stringResource(R.string.rules_on_device))
                                RuleRunsOn.UNMARKED, null -> {}
                            }
                        }
                    },
                    trailingContent = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            TextButton(onClick = { onMove(rule.id, -1) }, enabled = index > 0) { Text("↑") }
                            TextButton(onClick = { onMove(rule.id, 1) }, enabled = index < state.rules.lastIndex) {
                                Text("↓")
                            }
                            Switch(checked = rule.rule.enabled, onCheckedChange = { onEnabled(rule.id, it) })
                        }
                    },
                    modifier = Modifier.clickable { onOpen(rule) }.testTag("rule-${rule.id}"),
                )
                HorizontalDivider()
            }
        }
    }
}

/** One line: the server runs another filter script, with the explicit switch (M4-23). */
@Composable
private fun ServerRulesNoticeRow(
    notice: ServerRulesNotice,
    onSwitch: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("server-rules-notice"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text =
                if (notice.switchRequested) {
                    stringResource(R.string.rules_switch_pending)
                } else {
                    stringResource(R.string.rules_other_script, notice.scriptName)
                },
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )
        if (!notice.switchRequested) {
            TextButton(onClick = onSwitch) { Text(stringResource(R.string.rules_switch)) }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun RuleEditor(
    draft: RuleDraft,
    folders: List<FolderSummary>,
    onChange: (RuleDraft) -> Unit,
    onSave: (RuleDraft) -> Unit,
    onCancel: () -> Unit,
    onDelete: (() -> Unit)? = null,
    onRunNow: (() -> Unit)? = null,
) {
    var tried by rememberSaveable { mutableStateOf(false) }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(if (draft.id == null) R.string.rules_new else R.string.rules_edit)) },
                navigationIcon = { TextButton(onClick = onCancel) { Text(stringResource(R.string.rules_cancel)) } },
                actions = {
                    TextButton(onClick = {
                        tried = true
                        onSave(draft)
                    }) { Text(stringResource(R.string.rules_save)) }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (tried && draft.toInput(0) == null) {
                Text(stringResource(R.string.rules_incomplete), color = MaterialTheme.colorScheme.error)
            }
            OutlinedTextField(
                value = draft.name,
                onValueChange = { onChange(draft.copy(name = it)) },
                label = { Text(stringResource(R.string.rules_name)) },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Text(stringResource(R.string.rules_when), style = MaterialTheme.typography.titleSmall)
            Row {
                Choice(
                    stringResource(R.string.rules_match_all),
                    draft.matchAll,
                ) { onChange(draft.copy(matchAll = true)) }
                Choice(
                    stringResource(R.string.rules_match_any),
                    !draft.matchAll,
                ) { onChange(draft.copy(matchAll = false)) }
            }
            draft.conditions.forEachIndexed { i, c ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    KindMenu(
                        stringResource(c.kind.label),
                        ConditionKind.entries.map { it to stringResource(it.label) },
                    ) {
                        onChange(draft.copy(conditions = draft.conditions.replace(i, c.copy(kind = it))))
                    }
                    if (c.kind.needsText) {
                        OutlinedTextField(
                            value = c.text,
                            onValueChange = {
                                onChange(
                                    draft.copy(conditions = draft.conditions.replace(i, c.copy(text = it))),
                                )
                            },
                            singleLine = true,
                            label = {
                                Text(
                                    stringResource(
                                        if (c.kind ==
                                            ConditionKind.OLDER_THAN
                                        ) {
                                            R.string.rules_days
                                        } else {
                                            R.string.rules_contains
                                        },
                                    ),
                                )
                            },
                            modifier = Modifier.weight(1f),
                        )
                    } else {
                        Text("", Modifier.weight(1f))
                    }
                    TextButton(onClick = {
                        onChange(
                            draft.copy(
                                conditions =
                                    draft.conditions.filterIndexed { j, _ ->
                                        j !=
                                            i
                                    },
                            ),
                        )
                    }) {
                        Text(stringResource(R.string.rules_remove))
                    }
                }
            }
            OutlinedButton(onClick = { onChange(draft.copy(conditions = draft.conditions + ConditionDraft())) }) {
                Text(stringResource(R.string.rules_add_condition))
            }
            Text(stringResource(R.string.rules_then), style = MaterialTheme.typography.titleSmall)
            draft.actions.forEachIndexed { i, a ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    KindMenu(stringResource(a.kind.label), ActionKind.entries.map { it to stringResource(it.label) }) {
                        onChange(draft.copy(actions = draft.actions.replace(i, a.copy(kind = it))))
                    }
                    if (a.kind == ActionKind.MOVE) {
                        val current = folders.firstOrNull { it.id == a.folderId }?.let { folderName(it) }
                        KindMenu(
                            current ?: stringResource(R.string.rules_pick_folder),
                            folders.map {
                                it.id to
                                    folderName(it)
                            },
                        ) {
                            onChange(draft.copy(actions = draft.actions.replace(i, a.copy(folderId = it))))
                        }
                    }
                    Text("", Modifier.weight(1f))
                    TextButton(onClick = {
                        onChange(
                            draft.copy(
                                actions =
                                    draft.actions.filterIndexed {
                                        j,
                                        _,
                                        ->
                                        j != i
                                    },
                            ),
                        )
                    }) {
                        Text(stringResource(R.string.rules_remove))
                    }
                }
            }
            OutlinedButton(onClick = { onChange(draft.copy(actions = draft.actions + ActionDraft())) }) {
                Text(stringResource(R.string.rules_add_action))
            }
            SwitchLine(
                stringResource(R.string.rules_stop),
                draft.stopProcessing,
            ) { onChange(draft.copy(stopProcessing = it)) }
            SwitchLine(stringResource(R.string.rules_this_account), draft.thisAccountOnly) {
                onChange(draft.copy(thisAccountOnly = it))
            }
            SwitchLine(stringResource(R.string.rules_enabled), draft.enabled) { onChange(draft.copy(enabled = it)) }
            onRunNow?.let { Button(onClick = it) { Text(stringResource(R.string.rules_run_now)) } }
            onDelete?.let { TextButton(onClick = it) { Text(stringResource(R.string.rules_delete)) } }
        }
    }
}

private fun <T> List<T>.replace(
    index: Int,
    value: T,
): List<T> = mapIndexed { i, v -> if (i == index) value else v }

@Composable
private fun Choice(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
            Modifier
                .selectable(
                    selected = selected,
                    onClick = onClick,
                    role = Role.RadioButton,
                ).padding(end = 16.dp),
    ) {
        RadioButton(selected = selected, onClick = null)
        Text(label)
    }
}

@Composable
private fun SwitchLine(
    label: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

@Composable
private fun <T> KindMenu(
    label: String,
    options: List<Pair<T, String>>,
    onPick: (T) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Column {
        TextButton(onClick = { open = true }, modifier = Modifier.width(150.dp)) { Text(label, maxLines = 1) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            options.forEach { (value, text) ->
                DropdownMenuItem(text = { Text(text) }, onClick = {
                    open = false
                    onPick(value)
                })
            }
        }
    }
}
