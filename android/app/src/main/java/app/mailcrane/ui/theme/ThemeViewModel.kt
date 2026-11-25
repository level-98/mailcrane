package app.mailcrane.ui.theme

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.ThemeMode
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import javax.inject.Inject

/** The user's appearance settings, for the theme at the root of the app. */
data class Appearance(
    val mode: ThemeMode = ThemeMode.SYSTEM,
    val dynamicColor: Boolean = true,
    val darkMessageBodies: Boolean = false,
)

@HiltViewModel
class ThemeViewModel
    @Inject
    constructor(
        settings: AppSettings,
    ) : ViewModel() {
        val appearance: StateFlow<Appearance> =
            combine(settings.themeMode, settings.dynamicColor, settings.darkMessageBodies) { mode, dynamic, dark ->
                Appearance(mode, dynamic, dark)
            }.stateIn(viewModelScope, SharingStarted.Eagerly, Appearance())
    }
