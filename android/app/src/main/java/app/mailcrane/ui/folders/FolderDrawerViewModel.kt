package app.mailcrane.ui.folders

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.FolderRole
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MailboxCore
import app.mailcrane.core.SyncErrorKind
import app.mailcrane.core.SyncFailure
import app.mailcrane.core.UNIFIED_FOLDER_ID
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

data class FolderDrawerState(
    /** The account's folders in the order the core sorted them. */
    val folders: List<FolderSummary> = emptyList(),
    /** The folder whose messages are shown. */
    val selectedFolderId: Long? = null,
    /** No folders are stored yet and the first folder refresh is running. */
    val isLoading: Boolean = true,
    /** Refreshing the folders failed and none are stored, so there is nothing to show. */
    val loadError: SyncErrorKind? = null,
    /** Unread messages in every account's inbox, for "All inboxes" (M4-01). */
    val unifiedUnread: Int = 0,
    /** Whether to offer "Manage folders" (M4-22). */
    val canManageFolders: Boolean = false,
) {
    val selected: FolderSummary?
        get() =
            if (selectedFolderId == UNIFIED_FOLDER_ID) {
                // The Unified Inbox isn't a folder; it reads as one in the list header.
                FolderSummary(UNIFIED_FOLDER_ID, "", FolderRole.INBOX, 0u, unifiedUnread.toUInt())
            } else {
                folders.firstOrNull { it.id == selectedFolderId }
            }
}

/**
 * The folders of one account and which one is open. Shows the stored folders at once, refreshes the
 * list from the server, and re-reads it (with its unread counts) whenever the core reports a change.
 */
@HiltViewModel(assistedFactory = FolderDrawerViewModel.Factory::class)
class FolderDrawerViewModel
    @AssistedInject
    constructor(
        private val core: MailboxCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): FolderDrawerViewModel
        }

        private val _state = MutableStateFlow(FolderDrawerState())
        val state: StateFlow<FolderDrawerState> = _state.asStateFlow()

        private val reading = Mutex()

        init {
            viewModelScope.launch {
                // Unread counts follow message and folder changes. A sync sends many events; one
                // re-read after a burst is enough.
                core.events
                    .filter { it !is CoreEvent.SyncError }
                    .conflate()
                    .collect { reload() }
            }
            viewModelScope.launch {
                reload()
                refresh()
            }
        }

        /** Opens folder [folderId], if the account has it. */
        fun select(folderId: Long) {
            if (folderId == UNIFIED_FOLDER_ID || _state.value.folders.any { it.id == folderId }) {
                _state.update { it.copy(selectedFolderId = folderId) }
            }
        }

        /** The folder to open once the folders are read, from [startIn]. */
        private var pendingStart: Long? = null

        /**
         * Opens folder [folderId] at start ("Last used", #373): now if the folders are read, else once they
         * are. If the account no longer has it, the inbox stays open.
         */
        fun startIn(folderId: Long) {
            if (_state.value.folders.isEmpty()) {
                pendingStart = folderId
            } else {
                select(folderId)
            }
        }

        /** Opens the account's inbox (or its first folder), such as when leaving All inboxes. */
        fun openInbox() {
            _state.update { it.copy(selectedFolderId = defaultFolder(it.folders, null) ?: it.selectedFolderId) }
        }

        /** Tries again after the folder list couldn't be loaded. */
        fun retry() {
            _state.update { it.copy(isLoading = it.folders.isEmpty(), loadError = null) }
            viewModelScope.launch { refresh() }
        }

        /** Lists the folders on the server, then re-reads them; opens the inbox if nothing is open. */
        private suspend fun refresh() {
            try {
                val inbox = core.refreshFolders(accountId)
                reload(preferred = inbox)
                _state.update { it.copy(isLoading = false) }
            } catch (e: SyncFailure.Failed) {
                // With folders stored, keep showing them; the message list reports sync errors.
                failed(e.kind)
                _state.update { it.copy(isLoading = false) }
            }
        }

        /** Shows [kind] if there are no folders to show instead. */
        private fun failed(kind: SyncErrorKind) {
            // Another sync was saving at the same time; nothing to show (issue #282).
            if (kind == SyncErrorKind.BUSY) return
            _state.update { if (it.folders.isEmpty()) it.copy(loadError = kind) else it }
        }

        /** Re-reads the stored folders, keeping the open folder if it still exists. */
        private suspend fun reload(preferred: Long? = null) {
            reading.withLock {
                val folders =
                    try {
                        core.listFolders(accountId)
                    } catch (e: SyncFailure.Failed) {
                        failed(e.kind)
                        return
                    }
                val unified = core.unifiedUnreadCount()
                val manage = core.canManageFolders(accountId)
                // The start folder, the first time there are folders to look it up in (#373).
                val start = pendingStart?.takeIf { id -> folders.any { it.id == id } }
                if (folders.isNotEmpty()) pendingStart = null
                _state.update { current ->
                    val open =
                        current.selectedFolderId?.takeIf { id ->
                            id == UNIFIED_FOLDER_ID || folders.any { it.id == id }
                        }
                    current.copy(
                        folders = folders,
                        selectedFolderId = start ?: open ?: defaultFolder(folders, preferred),
                        isLoading = current.isLoading && folders.isEmpty(),
                        loadError = if (folders.isEmpty()) current.loadError else null,
                        unifiedUnread = unified,
                        canManageFolders = manage,
                    )
                }
            }
        }
    }

/** The folder to open when none is: the inbox, else [preferred], else the first one. */
internal fun defaultFolder(
    folders: List<FolderSummary>,
    preferred: Long?,
): Long? =
    folders.firstOrNull { it.role == FolderRole.INBOX }?.id
        ?: preferred?.takeIf { id -> folders.any { it.id == id } }
        ?: folders.firstOrNull()?.id
