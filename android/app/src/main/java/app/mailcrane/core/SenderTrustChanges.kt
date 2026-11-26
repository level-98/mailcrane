package app.mailcrane.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Tells open message screens that what decides the sender check changed (M5-08: an account's
 * provider authentication server name was saved or cleared). Message screens stay alive while
 * closed, so without this a lowered trust would only show after the app restarts.
 */
@Singleton
class SenderTrustChanges
    @Inject
    constructor() {
        private val _version = MutableStateFlow(0L)

        /** Goes up by one on every change. */
        val version: StateFlow<Long> = _version.asStateFlow()

        fun changed() = _version.update { it + 1 }
    }
