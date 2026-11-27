package app.mailcrane.settings

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
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
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

/** Settings → Reading (M4-15): when messages are marked read, and what shows after deleting or moving one. */
@HiltViewModel
class ReadingSettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
    ) : ViewModel() {
        val markReadMode: StateFlow<MarkReadMode> =
            settings.markReadMode.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), MarkReadMode.ON_OPEN)

        val afterMoveAction: StateFlow<AfterMoveAction> =
            settings.afterMoveAction.stateIn(
                viewModelScope,
                SharingStarted.WhileSubscribed(5_000),
                AfterMoveAction.LIST,
            )

        fun setMarkReadMode(mode: MarkReadMode) {
            viewModelScope.launch { settings.setMarkReadMode(mode) }
        }

        fun setAfterMoveAction(action: AfterMoveAction) {
            viewModelScope.launch { settings.setAfterMoveAction(action) }
        }
    }

@Composable
fun ReadingSection(viewModel: ReadingSettingsViewModel = hiltViewModel()) {
    val markRead by viewModel.markReadMode.collectAsStateWithLifecycle()
    val afterMove by viewModel.afterMoveAction.collectAsStateWithLifecycle()
    ReadingContent(markRead, viewModel::setMarkReadMode, afterMove, viewModel::setAfterMoveAction)
}

@Composable
internal fun ReadingContent(
    markRead: MarkReadMode,
    onMarkReadChange: (MarkReadMode) -> Unit,
    afterMove: AfterMoveAction,
    onAfterMoveChange: (AfterMoveAction) -> Unit,
) {
    ChoiceGroup(
        title = R.string.settings_mark_read,
        choices = MarkReadMode.entries.map { it to markReadLabel(it) },
        current = markRead,
        onChange = onMarkReadChange,
    )
    ChoiceGroup(
        title = R.string.settings_after_move,
        choices = AfterMoveAction.entries.map { it to stringResource(afterMoveLabel(it)) },
        current = afterMove,
        onChange = onAfterMoveChange,
    )
}

@Composable
private fun <T> ChoiceGroup(
    @StringRes title: Int,
    choices: List<Pair<T, String>>,
    current: T,
    onChange: (T) -> Unit,
) {
    Column(Modifier.fillMaxWidth().selectableGroup()) {
        Text(
            text = stringResource(title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        for ((value, label) in choices) {
            ListItem(
                leadingContent = { RadioButton(selected = current == value, onClick = null) },
                modifier =
                    Modifier.selectable(
                        selected = current == value,
                        role = Role.RadioButton,
                        onClick = { onChange(value) },
                    ),
            ) {
                Text(label)
            }
        }
    }
}

/** The delay comes from [MARK_READ_DELAY_MS], so the label follows it (#372 review). */
@Composable
private fun markReadLabel(mode: MarkReadMode): String =
    when (mode) {
        MarkReadMode.ON_OPEN -> stringResource(R.string.mark_read_on_open)
        MarkReadMode.AFTER_DELAY -> {
            val seconds = (MARK_READ_DELAY_MS / 1000).toInt()
            pluralStringResource(R.plurals.mark_read_after_delay, seconds, seconds)
        }
        MarkReadMode.MANUAL -> stringResource(R.string.mark_read_manual)
    }

private fun afterMoveLabel(action: AfterMoveAction): Int =
    when (action) {
        AfterMoveAction.NEXT -> R.string.after_move_next
        AfterMoveAction.PREVIOUS -> R.string.after_move_previous
        AfterMoveAction.LIST -> R.string.after_move_list
    }
