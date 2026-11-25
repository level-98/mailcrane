package app.mailcrane.settings

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

@HiltViewModel
class AdvancedSettingsViewModel
    @Inject
    constructor(
        private val settings: AppSettings,
    ) : ViewModel() {
        val backgroundDownload: StateFlow<BackgroundDownloadSetting> =
            settings.backgroundDownload.stateIn(
                viewModelScope,
                SharingStarted.WhileSubscribed(5_000),
                BackgroundDownloadSetting.WIFI_ONLY,
            )

        fun setBackgroundDownload(setting: BackgroundDownloadSetting) {
            viewModelScope.launch { settings.setBackgroundDownload(setting) }
        }
    }

/** "Advanced": "Download messages in the background" (M4-25). */
@Composable
fun AdvancedSection(viewModel: AdvancedSettingsViewModel = hiltViewModel()) {
    val setting by viewModel.backgroundDownload.collectAsStateWithLifecycle()
    AdvancedContent(setting, viewModel::setBackgroundDownload)
}

@Composable
fun AdvancedContent(
    backgroundDownload: BackgroundDownloadSetting,
    onBackgroundDownloadChange: (BackgroundDownloadSetting) -> Unit,
) {
    Column(Modifier.fillMaxWidth()) {
        Text(
            text = stringResource(R.string.settings_advanced_header),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp).semantics { heading() },
        )
        ListItem(
            supportingContent = { Text(stringResource(R.string.settings_background_download_summary)) },
        ) {
            Text(stringResource(R.string.settings_background_download))
        }
        Column(Modifier.fillMaxWidth().selectableGroup()) {
            for (choice in BackgroundDownloadSetting.entries) {
                ListItem(
                    leadingContent = { RadioButton(selected = backgroundDownload == choice, onClick = null) },
                    modifier =
                        Modifier.selectable(
                            selected = backgroundDownload == choice,
                            role = Role.RadioButton,
                            onClick = { onBackgroundDownloadChange(choice) },
                        ),
                ) {
                    Text(stringResource(backgroundDownloadLabel(choice)))
                }
            }
        }
    }
}

/** The label of a "Download messages in the background" choice. */
fun backgroundDownloadLabel(setting: BackgroundDownloadSetting): Int =
    when (setting) {
        BackgroundDownloadSetting.WIFI_ONLY -> R.string.settings_background_download_wifi
        BackgroundDownloadSetting.ALWAYS -> R.string.settings_background_download_always
        BackgroundDownloadSetting.NEVER -> R.string.settings_background_download_never
    }
