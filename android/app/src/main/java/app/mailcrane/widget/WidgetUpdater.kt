package app.mailcrane.widget

import android.content.Context
import androidx.glance.appwidget.updateAll
import app.mailcrane.core.CoreEvent
import app.mailcrane.core.MailboxCore
import app.mailcrane.settings.AppSettings
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch
import javax.inject.Inject
import javax.inject.Singleton

/** Refreshes the widget after mail changes (M4-11), at most once every couple of seconds. */
@Singleton
class WidgetUpdater
    @Inject
    constructor(
        @ApplicationContext private val context: Context,
        private val core: MailboxCore,
        private val settings: AppSettings,
    ) {
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

        @OptIn(FlowPreview::class)
        fun start() {
            scope.launch {
                core.events
                    .filter { it is CoreEvent.MessagesChanged || it is CoreEvent.FolderChanged }
                    .debounce(DEBOUNCE_MS)
                    .collect { refresh() }
            }
            // Turning the app lock or "hide content" on or off redraws the widget at once.
            scope.launch {
                combine(settings.appLockEnabled, settings.hideContentOutsideApp) { a, b -> a || b }
                    .distinctUntilChanged()
                    .drop(1)
                    .collect { refresh() }
            }
        }

        /** Redraws every inbox widget now. */
        suspend fun refresh() {
            try {
                InboxWidget().updateAll(context)
            } catch (_: IllegalStateException) {
                // No widget placed, or the launcher is not ready: nothing to do.
            }
        }

        private companion object {
            const val DEBOUNCE_MS = 2_000L
        }
    }
