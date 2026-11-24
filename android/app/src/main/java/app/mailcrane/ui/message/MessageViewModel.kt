package app.mailcrane.ui.message

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.InlinePart
import app.mailcrane.core.LinkCleaner
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageCore
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.settings.AppSettings
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

sealed interface MessageState {
    /** Reading the message, or downloading its body. */
    data object Loading : MessageState

    data class Ready(
        val message: MessageDetail,
        /** The user asked to load remote images for this message. */
        val remoteAllowed: Boolean,
    ) : MessageState {
        /** Whether to offer "Load remote images". */
        val showRemoteBanner: Boolean
            get() = !remoteAllowed && (message.body as? MessageBody.Html)?.remoteBlocked?.let { it > 0u } == true
    }

    data class Failed(
        val kind: SyncErrorKind,
    ) : MessageState
}

/**
 * One open message. Downloads the body if it isn't stored yet, marks the message as read once it
 * is shown, and re-reads it with remote images allowed when the user asks, for this message only.
 */
@HiltViewModel(assistedFactory = MessageViewModel.Factory::class)
class MessageViewModel
    @AssistedInject
    constructor(
        private val core: MessageCore,
        settings: AppSettings,
        private val linkCleaner: LinkCleaner,
        @Assisted private val messageId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(messageId: Long): MessageViewModel
        }

        private val _state = MutableStateFlow<MessageState>(MessageState.Loading)
        val state: StateFlow<MessageState> = _state.asStateFlow()

        // Until the setting is read, links are cleaned: the safe choice, and the default.
        private val cleanLinks = settings.cleanLinks.stateIn(viewModelScope, SharingStarted.Eagerly, true)

        private var remoteAllowed = false
        private var markedRead = false
        private var loading: Job? = null

        init {
            load()
        }

        /** Loads the message, downloading the body first if needed. Also the retry after a failure. */
        fun load() {
            loading?.cancel()
            _state.value = MessageState.Loading
            loading =
                viewModelScope.launch {
                    try {
                        var message = core.getMessage(messageId, remoteAllowed)
                        if (message.body is MessageBody.NotDownloaded) {
                            core.fetchBody(messageId)
                            message = core.getMessage(messageId, remoteAllowed)
                        }
                        _state.value = MessageState.Ready(message, remoteAllowed)
                        markRead(message)
                    } catch (e: SyncFailure.Failed) {
                        _state.value = MessageState.Failed(e.kind)
                    }
                }
        }

        /** The user tapped "Load remote images". Applies to this message only. */
        fun loadRemoteImages() {
            if (remoteAllowed) return
            remoteAllowed = true
            if (_state.value !is MessageState.Ready) return
            loading?.cancel()
            loading =
                viewModelScope.launch {
                    try {
                        _state.value = MessageState.Ready(core.getMessage(messageId, true), true)
                    } catch (e: SyncFailure.Failed) {
                        _state.value = MessageState.Failed(e.kind)
                    }
                }
        }

        /** The link to open for a tapped [url]: without tracking parameters when the setting is on. */
        fun linkToOpen(url: String): String = if (cleanLinks.value) linkCleaner.clean(url) else url

        /** Serves `mailcrane-cid:` images. Called on the web view's background thread. */
        fun inlinePart(contentId: String): InlinePart? =
            try {
                core.inlinePart(messageId, contentId)
            } catch (_: SyncFailure.Failed) {
                null
            }

        private suspend fun markRead(message: MessageDetail) {
            if (message.isSeen || markedRead) return
            markedRead = true
            try {
                core.markRead(messageId)
            } catch (_: SyncFailure.Failed) {
                // Only the unread dot stays; the message is shown either way. Allow another try.
                markedRead = false
            }
        }
    }
