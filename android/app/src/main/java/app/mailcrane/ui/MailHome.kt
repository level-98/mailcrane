package app.mailcrane.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.BoundsTransform
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionLayout
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.LocalResources
import app.mailcrane.R
import app.mailcrane.ui.compose.ComposeResult
import app.mailcrane.ui.compose.ComposeScreen
import app.mailcrane.ui.folders.MailboxScreen

/**
 * The signed-in part of the app: the mailbox, and the compose screen, which grows out of the compose
 * button with the theme's expressive spring (standard motion when the system asks for less motion).
 */
@OptIn(ExperimentalSharedTransitionApi::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun MailHome(accountId: Long) {
    var composing by rememberSaveable { mutableStateOf(false) }
    var notice by rememberSaveable { mutableStateOf<String?>(null) }
    val resources = LocalResources.current
    val motion = MaterialTheme.motionScheme
    val spatial = motion.defaultSpatialSpec<Rect>()
    val bounds = BoundsTransform { _, _ -> spatial }
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
                        composing = false
                    },
                    modifier = shared,
                )
            } else {
                MailboxScreen(
                    accountId = accountId,
                    onCompose = { composing = true },
                    composeFabModifier = shared,
                    notice = notice,
                    onNoticeShown = { notice = null },
                )
            }
        }
    }
}
