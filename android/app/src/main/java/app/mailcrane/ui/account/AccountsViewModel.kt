package app.mailcrane.ui.account

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.mailcrane.core.AccountSummary
import app.mailcrane.core.MailcraneCore
import app.mailcrane.core.RemoveAccountFailure
import app.mailcrane.settings.AppSettings
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
)

/**
 * The saved accounts and which one is shown (M4-01). The choice is remembered, so the app opens on the
 * same account after a restart.
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
            viewModelScope.launch { reload(preferred = settings.lastAccountId.first()) }
        }

        private suspend fun reload(preferred: Long?) {
            val accounts = core.listAccounts()
            val current =
                accounts.firstOrNull { it.id == preferred }?.id
                    ?: accounts.firstOrNull { it.id == _state.value.currentId }?.id
                    ?: accounts.firstOrNull()?.id
            _state.update { it.copy(accounts = accounts, currentId = current) }
            current?.let { settings.setLastAccountId(it) }
        }

        /** Shows account [id]. */
        fun select(id: Long) {
            if (_state.value.accounts?.any { it.id == id } != true) return
            _state.update { it.copy(currentId = id) }
            viewModelScope.launch { settings.setLastAccountId(id) }
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
                } catch (_: RemoveAccountFailure) {
                    _state.update { it.copy(removeFailed = true) }
                }
                reload(preferred = _state.value.currentId?.takeIf { it != id })
            }
        }
    }
