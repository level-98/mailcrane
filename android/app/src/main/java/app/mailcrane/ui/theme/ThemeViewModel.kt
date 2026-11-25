package app.mailcrane.ui.theme

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.contacts.ContactPhotos
import app.mailcrane.contacts.NoContactPhotos
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.MailboxDensity
import app.mailcrane.settings.ThemeMode
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
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
        photos: ContactPhotos,
    ) : ViewModel() {
        val appearance: StateFlow<Appearance> =
            combine(settings.themeMode, settings.dynamicColor, settings.darkMessageBodies) { mode, dynamic, dark ->
                Appearance(mode, dynamic, dark)
            }.stateIn(viewModelScope, SharingStarted.Eagerly, Appearance())

        /** "Mailbox density" for every message list (M4-24). */
        val mailboxDensity: StateFlow<MailboxDensity> =
            settings.mailboxDensity.stateIn(viewModelScope, SharingStarted.Eagerly, MailboxDensity.COMFORTABLE)

        /** Contact photos only while "Use photos from your contacts" is on (M4-24). */
        val contactPhotos: StateFlow<ContactPhotos> =
            settings.contactPhotos
                .map { on -> if (on) photos else NoContactPhotos }
                .stateIn(viewModelScope, SharingStarted.Eagerly, NoContactPhotos)
    }
