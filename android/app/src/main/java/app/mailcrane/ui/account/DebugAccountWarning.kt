package app.mailcrane.ui.account

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.res.stringResource
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.debug.DebugTools
import app.mailcrane.settings.AppSettings
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * Debug builds only (#559): their signing key is public, so an "update" signed with it could take over
 * the debug app's data. Account setup says so once. Release builds (no debug tools) never show it.
 */
@HiltViewModel
class DebugAccountWarningViewModel
    @Inject
    constructor(
        tools: DebugTools,
        private val settings: AppSettings,
    ) : ViewModel() {
        /** Whether to show the warning now. */
        val show: StateFlow<Boolean> =
            if (tools.available) {
                settings.debugAccountWarningSeen.map { !it }
            } else {
                flowOf(false)
            }.stateIn(viewModelScope, SharingStarted.Eagerly, false)

        /** The user read it: never again. */
        fun dismiss() {
            viewModelScope.launch { settings.setDebugAccountWarningSeen() }
        }
    }

/** The one-time "don't add a real account" dialog of debug builds (#559). */
@Composable
fun DebugAccountWarning(viewModel: DebugAccountWarningViewModel = hiltViewModel()) {
    val show by viewModel.show.collectAsStateWithLifecycle()
    if (!show) return
    AlertDialog(
        onDismissRequest = viewModel::dismiss,
        title = { Text(stringResource(R.string.debug_account_warning_title)) },
        text = { Text(stringResource(R.string.debug_account_warning_text)) },
        confirmButton = {
            TextButton(onClick = viewModel::dismiss) { Text(stringResource(R.string.debug_account_warning_ok)) }
        },
    )
}
