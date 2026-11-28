package app.mailcrane.ui.account

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.RemoveAccountFailure
import app.mailcrane.core.UNIFIED_FOLDER_ID
import app.mailcrane.settings.AppSettings
import app.mailcrane.settings.StartView
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import javax.inject.Inject

data class AccountsState(
    /** Null until the saved accounts are read. */
    val accounts: List<AccountSummary>? = null,
    /** The account shown; null when there are none. */
    val currentId: Long? = null,
    /** Removing an account failed. */
    val removeFailed: Boolean = false,
    /** Open the Unified Inbox first (M4-16); cleared by [AccountsViewModel.startShown]. */
    val startInUnified: Boolean = false,
    /** Open this folder of the account first ("Last used", #373); cleared by [AccountsViewModel.startShown]. */
    val startFolderId: Long? = null,
)

/** Where the app opens (M4-16): an account, whether in the Unified Inbox, and whether the setting must reset. */
data class StartChoice(
    val accountId: Long?,
    val unified: Boolean,
    /** The chosen account is gone: the setting goes back to "Last used". */
    val resetToLast: Boolean = false,
    /**
     * The folder of [accountId] to open ("Last used", #373). The folder list opens its inbox instead when the
     * account no longer has it.
     */
    val folderId: Long? = null,
)

/** Picks the start view from "Open the app on", the accounts [ids], and what was shown last. */
fun startChoice(
    view: StartView,
    ids: List<Long>,
    lastAccountId: Long,
    lastFolderId: Long,
): StartChoice {
    val last = lastAccountId.takeIf { it in ids } ?: ids.firstOrNull()
    // The Unified Inbox is only offered with two or more accounts (#373 review).
    val several = ids.size > 1
    return when (view) {
        StartView.Unified ->
            if (several) StartChoice(last, unified = true) else StartChoice(last, unified = false, resetToLast = true)
        // The folder shown last, kept in settings, so it survives a reboot or the app being swiped away
        // (#373, lead's decision). The account shown last is gone: All inboxes, or the only account left.
        StartView.Last ->
            when {
                lastAccountId !in ids -> StartChoice(last, unified = several && lastAccountId != 0L)
                lastFolderId == UNIFIED_FOLDER_ID -> StartChoice(last, unified = several)
                else -> StartChoice(last, unified = false, folderId = lastFolderId.takeIf { it > 0 })
            }
        is StartView.Account ->
            if (view.accountId in ids) {
                StartChoice(view.accountId, unified = false)
            } else {
                StartChoice(ids.firstOrNull(), unified = false, resetToLast = true)
            }
    }
}

/**
 * The saved accounts and which one is shown (M4-01). Opens where "Open the app on" says (M4-16): by default
 * the account and inbox shown last.
 */
@HiltViewModel
class AccountsViewModel
    @Inject
    constructor(
        private val core: MailcraneCore,
        private val settings: AppSettings,
    ) : ViewModel() {
        private val _state = MutableStateFlow(AccountsState())
        val state: StateFlow<AccountsState> = _state.asStateFlow()

        init {
            viewModelScope.launch {
                val accounts = core.listAccounts()
                val choice =
                    startChoice(
                        settings.startView.first(),
                        accounts.map { it.id },
                        settings.lastAccountId.first(),
                        settings.lastFolderId.first(),
                    )
                if (choice.resetToLast) settings.setStartView(StartView.Last)
                // In the same state update as the account, so no frame shows the inbox first (#373 review).
                reload(
                    preferred = choice.accountId,
                    listed = accounts,
                    startInUnified = choice.unified,
                    startFolderId = choice.folderId,
                )
            }
        }

        private suspend fun reload(
            preferred: Long?,
            listed: List<AccountSummary>? = null,
            startInUnified: Boolean? = null,
            startFolderId: Long? = null,
        ) {
            val accounts = listed ?: core.listAccounts()
            val current =
                accounts.firstOrNull { it.id == preferred }?.id
                    ?: accounts.firstOrNull { it.id == _state.value.currentId }?.id
                    ?: accounts.firstOrNull()?.id
            _state.update {
                it.copy(
                    accounts = accounts,
                    currentId = current,
                    startInUnified = startInUnified ?: it.startInUnified,
                    startFolderId = startFolderId ?: it.startFolderId,
                )
            }
            current?.let { settings.setLastAccountId(it) }
        }

        /** Shows account [id]. */
        fun select(id: Long) {
            if (_state.value.accounts?.any { it.id == id } != true) return
            // The start folder belongs to the account opened first; another one opens its inbox.
            _state.update { it.copy(currentId = id, startFolderId = null) }
            viewModelScope.launch { settings.setLastAccountId(id) }
        }

        /** The start view was shown: switching accounts later opens their inbox as usual. */
        fun startShown() {
            _state.update { it.copy(startInUnified = false, startFolderId = null) }
        }

        /** Folder [id] is shown ([UNIFIED_FOLDER_ID] for the Unified Inbox), for "Last used" (M4-16). */
        fun folderShown(id: Long) {
            viewModelScope.launch { settings.setLastFolderId(id) }
        }

        /** Something about the accounts changed, such as a colour (M4-20): load them again. */
        fun refresh() {
            viewModelScope.launch { reload(preferred = _state.value.currentId) }
        }

        /** A new account was saved: show it. */
        fun added(id: Long) {
            viewModelScope.launch { reload(preferred = id) }
        }

        /** Removes account [id] (the screen has asked first) and shows another one, if any. */
        fun remove(id: Long) {
            viewModelScope.launch {
                try {
                    core.removeAccount(id)
                    _state.update { it.copy(removeFailed = false) }
                    // The account the app opened on is gone, or the Unified Inbox no longer is offered
                    // (fewer than two accounts left): "Last used" from now on.
                    val left = _state.value.accounts?.count { it.id != id } ?: 0
                    val view = settings.startView.first()
                    if (view == StartView.Account(id) || (view == StartView.Unified && left < 2)) {
                        settings.setStartView(StartView.Last)
                    }
                } catch (_: RemoveAccountFailure) {
                    _state.update { it.copy(removeFailed = true) }
                }
                reload(preferred = _state.value.currentId?.takeIf { it != id })
            }
        }
    }
