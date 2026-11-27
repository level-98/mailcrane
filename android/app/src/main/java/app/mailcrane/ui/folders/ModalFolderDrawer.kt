package app.mailcrane.ui.folders

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.tween
import androidx.compose.material3.DrawerState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.MotionScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember

/** How long the folder drawer takes to open or close, or to settle after a drag. */
internal const val DRAWER_MOTION_MS = 250

/**
 * The folder drawer that slides in over the list on phones and small windows.
 *
 * It moves with a tween rather than the theme's spring (#551). Material's drawer takes a touch
 * during its animation as a grab of the drawer, so the touch never reaches the item under it.
 * The expressive spring keeps running, too small to see, for most of a second after the drawer
 * looks open, so the first tap on a folder was lost. A tween ends when it looks finished. The
 * drawer's content and the screen behind it keep the theme's own motion.
 */
@Composable
internal fun ModalFolderDrawer(
    drawerState: DrawerState,
    drawerContent: @Composable () -> Unit,
    content: @Composable () -> Unit,
) {
    val appMotion = MaterialTheme.motionScheme
    val drawerMotion = remember(appMotion) { TweenDrawerMotion(appMotion) }
    MaterialTheme(motionScheme = drawerMotion) {
        ModalNavigationDrawer(
            drawerState = drawerState,
            drawerContent = { MaterialTheme(motionScheme = appMotion, content = drawerContent) },
        ) {
            MaterialTheme(motionScheme = appMotion, content = content)
        }
    }
}

/**
 * [base], except for the two specs Material's drawer reads: default spatial (opening, and settling
 * after a drag) and fast effects (closing), both a [DRAWER_MOTION_MS] tween.
 */
private class TweenDrawerMotion(
    base: MotionScheme,
) : MotionScheme by base {
    override fun <T> defaultSpatialSpec(): FiniteAnimationSpec<T> = drawerTween()

    override fun <T> fastEffectsSpec(): FiniteAnimationSpec<T> = drawerTween()
}

private fun <T> drawerTween(): FiniteAnimationSpec<T> = tween(DRAWER_MOTION_MS, easing = FastOutSlowInEasing)
