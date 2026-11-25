package app.mailcrane.ui.compose

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.ComposeCore
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import javax.inject.Inject

/** Undo send: takes a waiting message back and saves it as a draft to reopen. */
@HiltViewModel
class UndoSendViewModel
    @Inject
    constructor(
        private val core: ComposeCore,
    ) : ViewModel() {
        /** Calls [onDraft] with the reopened draft's id, or [onTooLate] if it already started sending. */
        fun undo(
            outboxId: Long,
            onDraft: (Long) -> Unit,
            onTooLate: () -> Unit,
        ) {
            viewModelScope.launch {
                val draftId =
                    try {
                        core.cancelPending(outboxId)?.let { core.saveDraft(null, it) }
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        null
                    }
                if (draftId != null) onDraft(draftId) else onTooLate()
            }
        }
    }
