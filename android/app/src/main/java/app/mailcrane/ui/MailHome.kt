package app.mailcrane.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.BoundsTransform
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.unit.dp
import androidx.hilt.lifecycle.viewmodel.compose.hiltViewModel
import app.mailcrane.R
import app.mailcrane.ui.compose.ComposeResult
import app.mailcrane.ui.compose.ComposeScreen
import app.mailcrane.ui.compose.DraftRequest
import app.mailcrane.ui.compose.ReplyRequest
import app.mailcrane.ui.compose.UndoSendViewModel
import app.mailcrane.ui.compose.formatSendAt
import app.mailcrane.ui.folders.MailboxScreen
import kotlinx.coroutines.withTimeoutOrNull

/**
 * The signed-in part of the app: the mailbox, and the compose screen, which grows out of the compose
 * button with the theme's expressive spring (standard motion when the system asks for less motion).
 */
@OptIn(ExperimentalSharedTransitionApi::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun MailHome(accountId: Long) {
    var composing by rememberSaveable { mutableStateOf(false) }
    var reply by rememberSaveable { mutableStateOf<ReplyRequest?>(null) }
    var draft by rememberSaveable { mutableStateOf<DraftRequest?>(null) }
    var notice by rememberSaveable { mutableStateOf<String?>(null) }
    val resources = LocalResources.current
    // A sent message still waiting, as (outbox id, seconds), while Undo is offered.
    var undoable by rememberSaveable { mutableStateOf<Pair<Long, Int>?>(null) }
    val undoSnackbar = remember { SnackbarHostState() }
    val undoer: UndoSendViewModel = hiltViewModel()
    val pending = undoable
    LaunchedEffect(pending) {
        if (pending == null) return@LaunchedEffect
        val (outboxId, seconds) = pending
        // Offered for as long as the message waits; after that it's on its way.
        val result =
            withTimeoutOrNull(seconds * 1_000L) {
                undoSnackbar.showSnackbar(
                    message = resources.getString(R.string.undo_send_sending),
                    actionLabel = resources.getString(R.string.undo_send_undo),
                    duration = SnackbarDuration.Indefinite,
                )
            }
        undoable = null
        if (result == SnackbarResult.ActionPerformed) {
            undoer.undo(
                outboxId = outboxId,
                onDraft = {
                    draft = DraftRequest(draftId = it)
                    reply = null
                    composing = true
                },
                onTooLate = { notice = resources.getString(R.string.undo_send_too_late) },
            )
        }
    }
    val motion = MaterialTheme.motionScheme
    val spatial = motion.defaultSpatialSpec<Rect>()
    val bounds = BoundsTransform { _, _ -> spatial }
    Box(Modifier.fillMaxSize()) {
        SharedTransitionLayout {
            AnimatedContent(
                targetState = composing,
                transitionSpec = {
                    fadeIn(motion.defaultEffectsSpec()) togetherWith fadeOut(motion.fastEffectsSpec())
                },
                label = "compose",
            ) { open ->
                val shared =
                    Modifier.sharedBounds(
                        sharedContentState = rememberSharedContentState(key = "compose"),
                        animatedVisibilityScope = this@AnimatedContent,
                        boundsTransform = bounds,
                    )
                if (open) {
                    ComposeScreen(
                        accountId = accountId,
                        reply = reply,
                        draft = draft,
                        onDone = { result ->
                            if (result is ComposeResult.Sent) {
                                notice =
                                    if (result.rejected.isEmpty()) {
                                        resources.getString(R.string.compose_sent)
                                    } else {
                                        resources.getString(
                                            R.string.compose_sent_partly,
                                            result.rejected.joinToString(", "),
                                        )
                                    }
                            }
                            if (result is ComposeResult.Undoable) undoable = result.outboxId to result.seconds
                            if (result is ComposeResult.Scheduled) {
                                notice = resources.getString(R.string.schedule_done, formatSendAt(result.sendAt))
                            }
                            if (result is ComposeResult.Queued) {
                                notice = resources.getString(R.string.compose_queued)
                            }
                            if (result is ComposeResult.SavedDraft) {
                                notice = resources.getString(R.string.compose_draft_saved)
                            }
                            composing = false
                            reply = null
                            draft = null
                        },
                        modifier = shared,
                    )
                } else {
                    MailboxScreen(
                        accountId = accountId,
                        onCompose = {
                            reply = null
                            draft = null
                            composing = true
                        },
                        onReply = {
                            reply = it
                            draft = null
                            composing = true
                        },
                        onOpenDraft = {
                            draft = it
                            reply = null
                            composing = true
                        },
                        composeFabModifier = shared,
                        notice = notice,
                        onNoticeShown = { notice = null },
                    )
                }
            }
        }
        SnackbarHost(undoSnackbar, Modifier.align(Alignment.BottomCenter).padding(bottom = 88.dp))
    }
}
