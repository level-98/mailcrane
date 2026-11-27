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
 * The messages next to an open message in the list it was opened from, taken when it opened (M4-15). By id,
 * not position, so mail arriving or leaving above it later doesn't shift them (#372 review).
 */
data class Neighbours(
    val previous: Long?,
    val next: Long?,
)

/**
 * The list an open message was opened from, taken when it opened (#514): the folder (or the Unified Inbox),
 * the conversation if it was opened from one, and the order shown then.
 */
data class Origin(
    val folderId: Long,
    val threadId: Long?,
    val order: List<Long>,
)

/** The open message's [Origin], or null when it has none: nothing open, or opened from search. */
fun originOf(
    messageId: Long?,
    openedFromSearch: Boolean,
    folderId: Long?,
    threadId: Long?,
    order: List<Long>,
): Origin? = if (messageId == null || openedFromSearch || folderId == null) null else Origin(folderId, threadId, order)

/** [id]'s neighbours in [order], or null if it isn't there (opened from search, or from elsewhere). */
fun neighboursIn(
    order: List<Long>,
    id: Long?,
): Neighbours? {
    val i = if (id == null) -1 else order.indexOf(id)
    if (i < 0) return null
    return Neighbours(previous = order.getOrNull(i - 1), next = order.getOrNull(i + 1))
}

/**
 * The message to show after the open one was archived, deleted or moved (M4-15), or null for the list.
 * [neighbours] are the open message's, from when it opened; [current] is the same list as it is now, read
 * from the store (#514). A neighbour that list no longer holds is gone too, so the list shows instead.
 */
fun messageAfterLeaving(
    neighbours: Neighbours?,
    action: AfterMoveAction,
    current: List<Long>,
): Long? {
    val target =
        when (action) {
            AfterMoveAction.LIST -> return null
            AfterMoveAction.NEXT -> neighbours?.next
            AfterMoveAction.PREVIOUS -> neighbours?.previous
        }
    return target?.takeIf { it in current }
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
