package app.mailcrane.ui

import androidx.lifecycle.ViewModel
import app.mailcrane.core.MailcraneCore
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject

@HiltViewModel
class HelloViewModel
    @Inject
    constructor(
        core: MailcraneCore,
    ) : ViewModel() {
        /** The greeting from the core. Empty if the core call fails, so the app never crashes on it. */
        val greeting: String = runCatching { core.hello() }.getOrDefault("")
    }
