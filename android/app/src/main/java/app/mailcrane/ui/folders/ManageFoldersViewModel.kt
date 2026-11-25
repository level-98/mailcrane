package app.mailcrane.ui.folders

import androidx.annotation.StringRes
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.R
import app.mailcrane.core.FolderAdminCore
import app.mailcrane.core.FolderFailure
import app.mailcrane.core.FolderNameProblem
import app.mailcrane.core.FolderSummary
import app.mailcrane.core.MoveTargets
import app.mailcrane.core.SyncFailure
import dagger.assisted.Assisted
import dagger.assisted.AssistedFactory
import dagger.assisted.AssistedInject
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The dialog open on the Manage folders screen. */
sealed interface FolderDialog {
    /** A new folder under [parent], or at the top when null. */
    data class Create(
        val parent: FolderSummary?,
    ) : FolderDialog

    data class Rename(
        val folder: FolderSummary,
    ) : FolderDialog

    /** Choosing where [folder] goes; [targets] is null while they load. */
    data class Move(
        val folder: FolderSummary,
        val targets: MoveTargets?,
    ) : FolderDialog

    data class Delete(
        val folder: FolderSummary,
    ) : FolderDialog
}

/** A message to show, with an optional argument such as the folder separator. */
data class FolderMessage(
    @StringRes val text: Int,
    val arg: String? = null,
)

data class ManageFoldersState(
    val loaded: Boolean = false,
    val folders: List<FolderSummary> = emptyList(),
    val canManage: Boolean = false,
    val dialog: FolderDialog? = null,
    /** A change is being made on the server. */
    val busy: Boolean = false,
    /** Why the open dialog's change didn't happen. */
    val error: FolderMessage? = null,
)

/** The Manage folders screen (M4-22). The core checks names and rules; this only shows its answers. */
@HiltViewModel(assistedFactory = ManageFoldersViewModel.Factory::class)
class ManageFoldersViewModel
    @AssistedInject
    constructor(
        private val core: FolderAdminCore,
        @Assisted private val accountId: Long,
    ) : ViewModel() {
        @AssistedFactory
        interface Factory {
            fun create(accountId: Long): ManageFoldersViewModel
        }

        private val _state = MutableStateFlow(ManageFoldersState())
        val state: StateFlow<ManageFoldersState> = _state.asStateFlow()

        init {
            viewModelScope.launch { reload() }
        }

        private suspend fun reload() {
            try {
                val folders = core.listFolders(accountId)
                val canManage = core.canManageFolders(accountId)
                _state.update { it.copy(loaded = true, folders = folders, canManage = canManage) }
            } catch (_: SyncFailure) {
                _state.update { it.copy(loaded = true) }
            }
        }

        fun startCreate(parent: FolderSummary?) = open(FolderDialog.Create(parent))

        fun startRename(folder: FolderSummary) = open(FolderDialog.Rename(folder))

        fun startDelete(folder: FolderSummary) = open(FolderDialog.Delete(folder))

        fun startMove(folder: FolderSummary) {
            open(FolderDialog.Move(folder, null))
            viewModelScope.launch {
                try {
                    val targets = core.moveTargets(folder.id)
                    _state.update { s ->
                        val d = s.dialog
                        if (d is FolderDialog.Move && d.folder.id == folder.id) s.copy(dialog = d.copy(targets = targets)) else s
                    }
                } catch (_: SyncFailure) {
                    _state.update { it.copy(error = FolderMessage(R.string.folders_error_storage)) }
                }
            }
        }

        fun dismiss() {
            if (_state.value.busy) return
            _state.update { it.copy(dialog = null, error = null) }
        }

        fun confirmCreate(name: String) {
            val d = _state.value.dialog as? FolderDialog.Create ?: return
            run { core.createFolder(accountId, d.parent?.id, name) }
        }

        fun confirmRename(name: String) {
            val d = _state.value.dialog as? FolderDialog.Rename ?: return
            run { core.renameFolder(d.folder.id, name) }
        }

        /** Moves the folder under [parentId], or to the top when null. */
        fun confirmMove(parentId: Long?) {
            val d = _state.value.dialog as? FolderDialog.Move ?: return
            run { core.moveFolder(d.folder.id, parentId) }
        }

        fun confirmDelete() {
            val d = _state.value.dialog as? FolderDialog.Delete ?: return
            run { core.deleteFolder(d.folder.id) }
        }

        private fun open(dialog: FolderDialog) {
            if (_state.value.busy) return
            _state.update { it.copy(dialog = dialog, error = null) }
        }

        /** Makes the change; closes the dialog when it worked, otherwise says why and keeps it open. */
        private fun run(change: suspend () -> Unit) {
            if (_state.value.busy) return
            _state.update { it.copy(busy = true, error = null) }
            viewModelScope.launch {
                val error =
                    try {
                        change()
                        null
                    } catch (e: FolderFailure) {
                        message(e)
                    }
                if (error == null) {
                    reload()
                    _state.update { it.copy(busy = false, dialog = null) }
                } else {
                    _state.update { it.copy(busy = false, error = error) }
                }
            }
        }
    }

/** What to tell the user about [failure]. */
internal fun message(failure: FolderFailure): FolderMessage =
    when (failure) {
        is FolderFailure.InvalidName ->
            when (val p = failure.problem) {
                FolderNameProblem.Empty -> FolderMessage(R.string.folders_error_empty)
                FolderNameProblem.TooLong -> FolderMessage(R.string.folders_error_too_long)
                FolderNameProblem.BadCharacter -> FolderMessage(R.string.folders_error_bad_character)
                is FolderNameProblem.Separator -> FolderMessage(R.string.folders_error_separator, p.separator)
                FolderNameProblem.Reserved -> FolderMessage(R.string.folders_error_reserved)
            }
        is FolderFailure.SystemFolder -> FolderMessage(R.string.folders_error_system)
        is FolderFailure.HasSubfolders -> FolderMessage(R.string.folders_error_has_subfolders)
        is FolderFailure.AlreadyExists -> FolderMessage(R.string.folders_error_exists)
        is FolderFailure.IntoItself -> FolderMessage(R.string.folders_error_into_itself)
        is FolderFailure.NoSubfolders -> FolderMessage(R.string.folders_error_no_subfolders)
        is FolderFailure.NotFound -> FolderMessage(R.string.folders_error_not_found)
        is FolderFailure.Refused -> FolderMessage(R.string.folders_error_refused)
        is FolderFailure.Unsupported -> FolderMessage(R.string.folders_error_unsupported)
        is FolderFailure.Sync -> FolderMessage(R.string.folders_error_offline)
        is FolderFailure.Storage -> FolderMessage(R.string.folders_error_storage)
    }
