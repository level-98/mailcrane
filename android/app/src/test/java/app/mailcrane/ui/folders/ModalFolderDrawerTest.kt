package app.mailcrane.ui.folders

import android.app.Application
import androidx.compose.material3.Button
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.Text
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.ui.theme.MailcraneTheme
import kotlinx.coroutines.launch
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The first tap on a folder in the drawer opens it (#551). */
@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class ModalFolderDrawerTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun aFolderTappedOnceTheDrawerLooksOpenOpens() {
        var opened = 0
        compose.setContent {
            MailcraneTheme {
                val drawerState = rememberDrawerState(DrawerValue.Closed)
                val scope = rememberCoroutineScope()
                ModalFolderDrawer(
                    drawerState = drawerState,
                    drawerContent = {
                        ModalDrawerSheet {
                            NavigationDrawerItem(label = { Text("Sent") }, selected = false, onClick = { opened++ })
                        }
                    },
                ) {
                    Button(onClick = { scope.launch { drawerState.open() } }) { Text("Folders") }
                }
            }
        }
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Folders").performClick()
        // A moment after the drawer looks open. The theme's spring used to still be running here, and
        // Material's drawer takes a touch during its animation as a grab, so this tap was lost.
        compose.mainClock.advanceTimeBy(TAP_AFTER_MS)
        compose.onNodeWithText("Sent").performClick()
        compose.mainClock.advanceTimeBy(100)
        assertEquals("the first tap opened the folder", 1, opened)
    }

    private companion object {
        const val TAP_AFTER_MS = 350L
    }
}
