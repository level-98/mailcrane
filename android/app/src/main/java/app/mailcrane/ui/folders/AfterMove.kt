package app.mailcrane.ui.folders

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.settings.AfterMoveAction
import app.mailcrane.settings.AppSettings
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import javax.inject.Inject

/**
 * The message to show after message [id] was archived, deleted or moved (M4-15), or null for the list.
 *
 * [order] is the list the message was opened from, as last shown; [openedIndex] is where [id] was in it
 * when it opened (-1 if it wasn't there, e.g. opened from search). The list may already have dropped [id]
 * (side by side, it refreshes at once); then the message after it has moved up into [openedIndex].
 */
fun messageAfterLeaving(
    order: List<Long>,
    id: Long,
    openedIndex: Int,
    action: AfterMoveAction,
): Long? {
    val current = order.indexOf(id)
    val target =
        when (action) {
            AfterMoveAction.LIST -> return null
            AfterMoveAction.NEXT -> if (current >= 0) current + 1 else openedIndex
            AfterMoveAction.PREVIOUS -> if (current >= 0) current - 1 else openedIndex - 1
        }
    if (current < 0 && openedIndex < 0) return null
    return order.getOrNull(target)?.takeIf { it != id }
}

/** "After deleting or moving" for the mailbox screen. */
@HiltViewModel
class AfterMoveViewModel
    @Inject
    constructor(
        settings: AppSettings,
    ) : ViewModel() {
        val action: StateFlow<AfterMoveAction> =
            settings.afterMoveAction.stateIn(viewModelScope, SharingStarted.Eagerly, AfterMoveAction.LIST)
    }
