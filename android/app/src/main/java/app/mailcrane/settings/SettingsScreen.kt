package app.mailcrane.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.ui.theme.dynamicColorAvailable
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
    ) : ViewModel() {
        val cleanLinks: StateFlow<Boolean> =
            settings.cleanLinks.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)

        fun setCleanLinks(enabled: Boolean) {
            viewModelScope.launch { settings.setCleanLinks(enabled) }
        }

        val confirmLinks: StateFlow<Boolean> =
            settings.confirmLinks.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)

        fun setConfirmLinks(enabled: Boolean) {
            viewModelScope.launch { settings.setConfirmLinks(enabled) }
        }

        val undoSendSeconds: StateFlow<Int> =
            settings.undoSendSeconds.stateIn(
                viewModelScope,
                SharingStarted.WhileSubscribed(5_000),
                DEFAULT_UNDO_SEND_SECONDS,
            )

        fun setUndoSendSeconds(seconds: Int) {
            viewModelScope.launch { settings.setUndoSendSeconds(seconds) }
        }

        val themeMode: StateFlow<ThemeMode> =
            settings.themeMode.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), ThemeMode.SYSTEM)
        val dynamicColor: StateFlow<Boolean> =
            settings.dynamicColor.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)
        val darkMessageBodies: StateFlow<Boolean> =
            settings.darkMessageBodies.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

        fun setThemeMode(mode: ThemeMode) {
            viewModelScope.launch { settings.setThemeMode(mode) }
        }

        fun setDynamicColor(enabled: Boolean) {
            viewModelScope.launch { settings.setDynamicColor(enabled) }
        }

        val hideContentOutsideApp: StateFlow<Boolean> =
            settings.hideContentOutsideApp.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

        fun setHideContentOutsideApp(hide: Boolean) {
            viewModelScope.launch { settings.setHideContentOutsideApp(hide) }
        }

        val groupByConversation: StateFlow<Boolean> =
            settings.groupByConversation.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)

        fun setGroupByConversation(enabled: Boolean) {
            viewModelScope.launch { settings.setGroupByConversation(enabled) }
        }

        fun setDarkMessageBodies(enabled: Boolean) {
            viewModelScope.launch { settings.setDarkMessageBodies(enabled) }
        }

        val swipeRight: StateFlow<SwipeAction> =
            settings.swipeRight.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), DEFAULT_SWIPE_RIGHT)
        val swipeLeft: StateFlow<SwipeAction> =
            settings.swipeLeft.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), DEFAULT_SWIPE_LEFT)

        fun setSwipeRight(action: SwipeAction) {
            viewModelScope.launch { settings.setSwipeRight(action) }
        }

        fun setSwipeLeft(action: SwipeAction) {
            viewModelScope.launch { settings.setSwipeLeft(action) }
        }
    }

/** The app's settings. */
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    accountId: Long? = null,
    onOpenRules: (() -> Unit)? = null,
    accounts: List<app.mailcrane.core.AccountSummary> = emptyList(),
    onAddAccount: (() -> Unit)? = null,
    onRemoveAccount: (Long) -> Unit = {},
    onAccountsChanged: () -> Unit = {},
    viewModel: SettingsViewModel = hiltViewModel(),
) {
    val cleanLinks by viewModel.cleanLinks.collectAsStateWithLifecycle()
    val confirmLinks by viewModel.confirmLinks.collectAsStateWithLifecycle()
    val undoSend by viewModel.undoSendSeconds.collectAsStateWithLifecycle()
    val themeMode by viewModel.themeMode.collectAsStateWithLifecycle()
    val dynamicColor by viewModel.dynamicColor.collectAsStateWithLifecycle()
    val darkBodies by viewModel.darkMessageBodies.collectAsStateWithLifecycle()
    val grouped by viewModel.groupByConversation.collectAsStateWithLifecycle()
    val hideOutside by viewModel.hideContentOutsideApp.collectAsStateWithLifecycle()
    val swipeRight by viewModel.swipeRight.collectAsStateWithLifecycle()
    val swipeLeft by viewModel.swipeLeft.collectAsStateWithLifecycle()
    SettingsContent(
        cleanLinks = cleanLinks,
        onCleanLinksChange = viewModel::setCleanLinks,
        confirmLinks = confirmLinks,
        onConfirmLinksChange = viewModel::setConfirmLinks,
        onBack = onBack,
        undoSendSeconds = undoSend,
        onUndoSendChange = viewModel::setUndoSendSeconds,
        appearance =
            AppearanceSettings(
                mode = themeMode,
                dynamicColor = dynamicColor,
                darkMessageBodies = darkBodies,
                groupByConversation = grouped,
                onGroupByConversationChange = viewModel::setGroupByConversation,
                onModeChange = viewModel::setThemeMode,
                onDynamicColorChange = viewModel::setDynamicColor,
                onDarkMessageBodiesChange = viewModel::setDarkMessageBodies,
            ),
        swipe =
            SwipeSettings(
                right = swipeRight,
                left = swipeLeft,
                onRightChange = viewModel::setSwipeRight,
                onLeftChange = viewModel::setSwipeLeft,
            ),
    ) {
        if (accountId != null) {
            SignatureSection(accountId)
            SaveSentSection(accountId)
            AccountColorSection(accountId, onChanged = onAccountsChanged)
        }
        if (onAddAccount != null) {
            AccountsSection(accounts, onAddAccount, onRemoveAccount)
        }
        NotificationsSection()
        AppLockSection()
        SwitchRow(
            title = R.string.settings_hide_outside,
            summary = R.string.settings_hide_outside_summary,
            checked = hideOutside,
            onChange = viewModel::setHideContentOutsideApp,
        )
        onOpenRules?.let { open ->
            androidx.compose.material3.ListItem(
                headlineContent = { Text(stringResource(R.string.rules_settings_row)) },
                supportingContent = { Text(stringResource(R.string.rules_settings_summary)) },
                modifier = Modifier.clickable(onClick = open),
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsContent(
    cleanLinks: Boolean,
    onCleanLinksChange: (Boolean) -> Unit,
    onBack: () -> Unit,
    confirmLinks: Boolean = true,
    onConfirmLinksChange: (Boolean) -> Unit = {},
    undoSendSeconds: Int = DEFAULT_UNDO_SEND_SECONDS,
    onUndoSendChange: (Int) -> Unit = {},
    appearance: AppearanceSettings = AppearanceSettings(),
    swipe: SwipeSettings = SwipeSettings(),
    accountSection: @Composable () -> Unit = {},
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            painter = painterResource(R.drawable.ic_mc_back),
                            contentDescription = stringResource(R.string.settings_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())) {
            // The whole row is one switch for TalkBack and touch; the Switch itself stays silent.
            ListItem(
                supportingContent = { Text(stringResource(R.string.settings_clean_links_summary)) },
                trailingContent = { Switch(checked = cleanLinks, onCheckedChange = null) },
                modifier =
                    Modifier.toggleable(value = cleanLinks, role = Role.Switch, onValueChange = onCleanLinksChange),
            ) {
                Text(stringResource(R.string.settings_clean_links))
            }
            ListItem(
                supportingContent = { Text(stringResource(R.string.settings_confirm_links_summary)) },
                trailingContent = { Switch(checked = confirmLinks, onCheckedChange = null) },
                modifier =
                    Modifier.toggleable(
                        value = confirmLinks,
                        role = Role.Switch,
                        onValueChange = onConfirmLinksChange,
                    ),
            ) {
                Text(stringResource(R.string.settings_confirm_links))
            }
            AppearanceSection(appearance)
            UndoSendSection(undoSendSeconds, onUndoSendChange)
            SwipeSection(R.string.settings_swipe_right, swipe.right, swipe.onRightChange)
            SwipeSection(R.string.settings_swipe_left, swipe.left, swipe.onLeftChange)
            accountSection()
        }
    }
}

/** "Undo send": how long a sent message waits, so it can be taken back. */
@Composable
private fun UndoSendSection(
    seconds: Int,
    onChange: (Int) -> Unit,
) {
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(R.string.settings_undo_send_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for (choice in UNDO_SEND_CHOICES) {
            ListItem(
                leadingContent = { RadioButton(selected = seconds == choice, onClick = null) },
                modifier =
                    Modifier.selectable(
                        selected = seconds == choice,
                        role = Role.RadioButton,
                        onClick = { onChange(choice) },
                    ),
            ) {
                Text(
                    if (choice == 0) {
                        stringResource(R.string.settings_undo_send_off)
                    } else {
                        pluralStringResource(R.plurals.settings_undo_send_seconds, choice, choice)
                    },
                )
            }
        }
    }
}

/** The appearance settings and what changing them does. */
data class AppearanceSettings(
    val mode: ThemeMode = ThemeMode.SYSTEM,
    val dynamicColor: Boolean = true,
    val darkMessageBodies: Boolean = false,
    val onModeChange: (ThemeMode) -> Unit = {},
    val onDynamicColorChange: (Boolean) -> Unit = {},
    val onDarkMessageBodiesChange: (Boolean) -> Unit = {},
    val groupByConversation: Boolean = true,
    val onGroupByConversationChange: (Boolean) -> Unit = {},
)

@Composable
private fun AppearanceSection(a: AppearanceSettings) {
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(R.string.settings_appearance_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for ((mode, label) in THEME_CHOICES) {
            ListItem(
                leadingContent = { RadioButton(selected = a.mode == mode, onClick = null) },
                modifier =
                    Modifier.selectable(
                        selected = a.mode == mode,
                        role = Role.RadioButton,
                        onClick = { a.onModeChange(mode) },
                    ),
            ) {
                Text(stringResource(label))
            }
        }
    }
    // Hidden where the system has no wallpaper colours (before Android 12).
    if (dynamicColorAvailable) {
        SwitchRow(
            title = R.string.settings_dynamic_color,
            summary = R.string.settings_dynamic_color_summary,
            checked = a.dynamicColor,
            onChange = a.onDynamicColorChange,
        )
    }
    SwitchRow(
        title = R.string.settings_dark_bodies,
        summary = R.string.settings_dark_bodies_summary,
        checked = a.darkMessageBodies,
        onChange = a.onDarkMessageBodiesChange,
    )
    SwitchRow(
        title = R.string.settings_group_threads,
        summary = R.string.settings_group_threads_summary,
        checked = a.groupByConversation,
        onChange = a.onGroupByConversationChange,
    )
}

private val THEME_CHOICES =
    listOf(
        ThemeMode.SYSTEM to R.string.settings_theme_system,
        ThemeMode.LIGHT to R.string.settings_theme_light,
        ThemeMode.DARK to R.string.settings_theme_dark,
    )

/** One switch as a whole row, for touch and TalkBack; the Switch itself stays silent. */
@Composable
private fun SwitchRow(
    title: Int,
    summary: Int,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
) {
    ListItem(
        supportingContent = { Text(stringResource(summary)) },
        trailingContent = { Switch(checked = checked, onCheckedChange = null) },
        modifier = Modifier.toggleable(value = checked, role = Role.Switch, onValueChange = onChange),
    ) {
        Text(stringResource(title))
    }
}

/** The two swipe settings and what changing them does. */
data class SwipeSettings(
    val right: SwipeAction = DEFAULT_SWIPE_RIGHT,
    val left: SwipeAction = DEFAULT_SWIPE_LEFT,
    val onRightChange: (SwipeAction) -> Unit = {},
    val onLeftChange: (SwipeAction) -> Unit = {},
)

@Composable
private fun SwipeSection(
    title: Int,
    current: SwipeAction,
    onChange: (SwipeAction) -> Unit,
) {
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for (action in SwipeAction.entries) {
            ListItem(
                leadingContent = { RadioButton(selected = current == action, onClick = null) },
                modifier =
                    Modifier.selectable(
                        selected = current == action,
                        role = Role.RadioButton,
                        onClick = { onChange(action) },
                    ),
            ) {
                Text(stringResource(swipeLabel(action)))
            }
        }
    }
}

/** The name of a swipe choice. */
fun swipeLabel(action: SwipeAction): Int =
    when (action) {
        SwipeAction.NONE -> R.string.swipe_none
        SwipeAction.ARCHIVE -> R.string.swipe_archive
        SwipeAction.DELETE -> R.string.swipe_delete
        SwipeAction.READ_UNREAD -> R.string.swipe_read_unread
        SwipeAction.FLAG -> R.string.swipe_flag
        SwipeAction.MOVE -> R.string.swipe_move
    }
