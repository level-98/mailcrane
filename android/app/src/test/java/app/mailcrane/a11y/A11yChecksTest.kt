package app.mailcrane.a11y

import android.app.Application
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.mailcrane.R
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** [assertAccessible] catches each kind of problem it claims to, so a passing screen means something. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class A11yChecksTest {
    @get:Rule
    val compose = createComposeRule()

    private fun problems(): String =
        assertThrows(AssertionError::class.java) { compose.assertAccessible("sample") }.message.orEmpty()

    @Test
    fun aGoodScreenPasses() {
        compose.setContent {
            Column {
                Text("Title", modifier = Modifier.semantics { heading() })
                IconButton(onClick = {}) {
                    Icon(painterResource(R.drawable.ic_mc_back), contentDescription = "Back")
                }
            }
        }
        compose.assertAccessible("good")
    }

    @Test
    fun anUnlabelledIconButtonIsCaught() {
        compose.setContent {
            Column {
                Text("Title", modifier = Modifier.semantics { heading() })
                IconButton(onClick = {}) { Icon(painterResource(R.drawable.ic_mc_back), contentDescription = null) }
            }
        }
        assertTrue(problems().contains("unlabelled control"))
    }

    @Test
    fun aSmallTouchTargetIsCaught() {
        compose.setContent {
            Column {
                Text("Title", modifier = Modifier.semantics { heading() })
                Box(Modifier.size(24.dp).clickable {}) { Text("x") }
            }
        }
        assertTrue(problems().contains("touch target under 48dp"))
    }

    @Test
    fun textCutOffAndAMissingHeadingAreCaught() {
        compose.setContent {
            Box(Modifier.height(20.dp)) {
                Text("A line of text far too tall for its box", fontSize = 40.sp)
            }
        }
        val found = problems()
        assertTrue(found, found.contains("text cut off"))
        assertTrue(found, found.contains("no heading"))
    }
}
