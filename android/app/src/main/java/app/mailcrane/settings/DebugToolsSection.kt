package app.mailcrane.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.debug.DebugTools
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import javax.inject.Inject

/** Where "Load test mail" stands. */
sealed interface TestMailState {
    data object Idle : TestMailState

    data object Loading : TestMailState

    data class Loaded(
        val count: Int,
    ) : TestMailState

    data object Failed : TestMailState
}

@HiltViewModel
class DebugToolsViewModel
    @Inject
    constructor(
        private val tools: DebugTools,
    ) : ViewModel() {
        /** Whether to show the section at all: debug builds only. */
        val available: Boolean get() = tools.available

        private val _state = MutableStateFlow<TestMailState>(TestMailState.Idle)
        val state: StateFlow<TestMailState> = _state.asStateFlow()

        /** Loads the test mail; [onLoaded] runs after a success, to refresh the account list. */
        fun loadFixtures(onLoaded: () -> Unit) {
            if (!tools.available || _state.value == TestMailState.Loading) return
            _state.value = TestMailState.Loading
            viewModelScope.launch {
                _state.value =
                    try {
                        TestMailState.Loaded(tools.loadFixtures()).also { onLoaded() }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        TestMailState.Failed
                    }
            }
        }
    }

/** "Debug tools" (#289), shown only in debug builds: "Load test mail". */
@Composable
fun DebugToolsSection(
    onAccountsChanged: () -> Unit,
    viewModel: DebugToolsViewModel = hiltViewModel(),
) {
    if (!viewModel.available) return
    val state by viewModel.state.collectAsStateWithLifecycle()
    Column(Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.settings_debug_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        ListItem(
            headlineContent = { Text(stringResource(R.string.settings_load_test_mail)) },
            supportingContent = {
                val current = state
                Text(
                    when (current) {
                        TestMailState.Idle -> stringResource(R.string.settings_load_test_mail_summary)
                        TestMailState.Loading -> stringResource(R.string.settings_load_test_mail_loading)
                        is TestMailState.Loaded ->
                            pluralStringResource(R.plurals.settings_load_test_mail_done, current.count, current.count)
                        TestMailState.Failed -> stringResource(R.string.settings_load_test_mail_failed)
                    },
                    color =
                        if (current == TestMailState.Failed) {
                            MaterialTheme.colorScheme.error
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                )
            },
            modifier =
                Modifier.clickable(enabled = state != TestMailState.Loading) {
                    viewModel.loadFixtures(onAccountsChanged)
                },
        )
    }
}
