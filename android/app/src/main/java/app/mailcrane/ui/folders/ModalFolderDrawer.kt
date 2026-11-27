package app.mailcrane.ui.folders

import androidx.compose.material3.DrawerState
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.runtime.Composable

/** The folder drawer that slides in over the list on phones and small windows. */
@Composable
internal fun ModalFolderDrawer(
    drawerState: DrawerState,
    drawerContent: @Composable () -> Unit,
    content: @Composable () -> Unit,
) {
    ModalNavigationDrawer(drawerState = drawerState, drawerContent = drawerContent, content = content)
}
