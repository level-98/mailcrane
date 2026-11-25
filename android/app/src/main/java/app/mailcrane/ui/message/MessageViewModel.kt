package app.mailcrane.ui.message

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.ActionFailure
import app.mailcrane.core.InlinePart
import app.mailcrane.core.LinkCleaner
import app.mailcrane.core.MessageAction
import app.mailcrane.core.MessageBody
import app.mailcrane.core.MessageCore
import app.mailcrane.core.MessageDetail
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.UnsubscribeFailure
import app.mailcrane.core.UnsubscribeInfo
import app.mailcrane.settings.AppSettings
import app.mailcrane.ui.compose.ComposePrefill
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

/** One-off things the message screen does after an action. */
sealed interface MessageEffect {
    /** The message left this folder (archived, deleted, moved) or was marked unread: close it. */
    data object Close : MessageEffect

    /** The action couldn't be applied; nothing changed. */
    data class ActionRefused(
        val reason: ActionRefusal,
    ) : MessageEffect

    /** Open compose with an unsubscribe email for the user to send. */
    data class ComposeUnsubscribe(
        val prefill: ComposePrefill,
    ) : MessageEffect

    /** Open the sender's unsubscribe page in the browser (the user already confirmed). */
    data class OpenUnsubscribePage(
        val url: String,
    ) : MessageEffect

    /** The one-click unsubscribe request finished. */
    data class Unsubscribed(
        val success: Boolean,
    ) : MessageEffect
}

/** How "Unsubscribe" will work for a message, best first (M4-10). */
enum class UnsubscribeMethod {
    /** Mailcrane tells the sender's server directly (RFC 8058). */
    ONE_CLICK,

    /** Mailcrane prepares an email for the user to send. */
    EMAIL,

    /** The sender's page opens in the browser. */
    WEB,
}

/** The method used for [info], or null if it offers nothing. */
fun unsubscribeMethod(info: UnsubscribeInfo): UnsubscribeMethod? =
    when {
        info.oneClick -> UnsubscribeMethod.ONE_CLICK
        info.mailto != null -> UnsubscribeMethod.EMAIL
        info.webUrl != null -> UnsubscribeMethod.WEB
        else -> null
    }

/** Why an action couldn't be applied, in terms the screen can explain. */
enum class ActionRefusal {
    /** The account has no Archive folder. */
    NO_ARCHIVE_FOLDER,

    /** The message or folder is gone, or the database failed. */
    FAILED,
}

sealed interface MessageState {
    /** Reading the message, or downloading its body. */
    data object Loading : MessageState

    data class Ready(
        val message: MessageDetail,
        /** The user asked to load remote images for this message. */
        val remoteAllowed: Boolean,
        /** How the message can be unsubscribed from (M4-10), if at all. */
        val unsubscribe: UnsubscribeInfo? = null,
        /** A one-click unsubscribe request is running. */
        val unsubscribing: Boolean = false,
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

        private val _effects = Channel<MessageEffect>(Channel.BUFFERED)

        /** Closing the screen, or explaining a refused action. */
        val effects: Flow<MessageEffect> = _effects.receiveAsFlow()

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
                        _state.value = MessageState.Ready(message, remoteAllowed, core.unsubscribeOptions(messageId))
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
                        val unsubscribe = (_state.value as? MessageState.Ready)?.unsubscribe
                        _state.value = MessageState.Ready(core.getMessage(messageId, true), true, unsubscribe)
                    } catch (e: SyncFailure.Failed) {
                        _state.value = MessageState.Failed(e.kind)
                    }
                }
        }

        /** Moves the message to the account's Archive folder and closes it. */
        fun archive() = act(MessageAction.Archive, close = true)

        /** Moves the message to Trash (or deletes it for good if it's in Trash) and closes it. */
        fun delete() = act(MessageAction.Delete, close = true)

        /** Moves the message to folder [folderId] and closes it. */
        fun moveTo(folderId: Long) = act(MessageAction.Move(folderId), close = true)

        /** Marks the message unread and closes it, so it isn't marked read again at once. */
        fun markUnread() = act(MessageAction.MarkUnread, close = true)

        /**
         * Unsubscribes the way [unsubscribeMethod] says. Call only after the user confirmed: one-click
         * sends the request now; the email and web methods hand over to compose or the browser.
         */
        fun unsubscribe() {
            val ready = _state.value as? MessageState.Ready ?: return
            val info = ready.unsubscribe ?: return
            if (ready.unsubscribing) return
            when (unsubscribeMethod(info)) {
                UnsubscribeMethod.ONE_CLICK -> {
                    _state.value = ready.copy(unsubscribing = true)
                    viewModelScope.launch {
                        val ok =
                            try {
                                core.oneClickUnsubscribe(messageId)
                                true
                            } catch (_: UnsubscribeFailure) {
                                false
                            }
                        (_state.value as? MessageState.Ready)?.let {
                            // Done: no button any more. Failed: fall back to the other methods.
                            val left = if (ok) null else it.unsubscribe?.copy(oneClick = false)
                            _state.value = it.copy(unsubscribing = false, unsubscribe = left)
                        }
                        _effects.send(MessageEffect.Unsubscribed(ok))
                    }
                }
                UnsubscribeMethod.EMAIL -> {
                    val m = info.mailto ?: return
                    viewModelScope.launch {
                        _effects.send(
                            MessageEffect.ComposeUnsubscribe(
                                ComposePrefill(
                                    to = m.address,
                                    subject = m.subject ?: "unsubscribe",
                                    body = m.body ?: "",
                                ),
                            ),
                        )
                    }
                }
                UnsubscribeMethod.WEB -> {
                    val url = info.webUrl ?: return
                    viewModelScope.launch { _effects.send(MessageEffect.OpenUnsubscribePage(url)) }
                }
                null -> {}
            }
        }

        /** Flags or unflags the message; the screen shows the new state at once. */
        fun toggleFlag() {
            val ready = _state.value as? MessageState.Ready ?: return
            val flagged = !ready.message.isFlagged
            _state.value = ready.copy(message = ready.message.copy(isFlagged = flagged))
            act(if (flagged) MessageAction.Flag else MessageAction.Unflag, close = false) {
                // Refused: show the flag as it was.
                (_state.value as? MessageState.Ready)?.let {
                    _state.value = it.copy(message = it.message.copy(isFlagged = !flagged))
                }
            }
        }

        private fun act(
            action: MessageAction,
            close: Boolean,
            onRefused: () -> Unit = {},
        ) {
            viewModelScope.launch {
                try {
                    core.applyAction(messageId, action)
                    if (close) _effects.send(MessageEffect.Close)
                } catch (e: ActionFailure) {
                    onRefused()
                    val reason =
                        when (e) {
                            is ActionFailure.NoArchiveFolder -> ActionRefusal.NO_ARCHIVE_FOLDER
                            else -> ActionRefusal.FAILED
                        }
                    _effects.send(MessageEffect.ActionRefused(reason))
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
