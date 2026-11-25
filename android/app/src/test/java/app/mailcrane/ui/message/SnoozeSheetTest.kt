package app.mailcrane.ui.message

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import app.mailcrane.core.SnoozeInfo
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZonedDateTime

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class SnoozeSheetTest {
    @get:Rule
    val compose = createComposeRule()

    // Wednesday 25 November 2026, 14:00 in London.
    private val now = ZonedDateTime.of(LocalDateTime.parse("2026-11-25T14:00"), ZoneId.of("Europe/London"))

    @Test
    fun presetsShowTheirTimesAndPick() {
        var picked: Long? = null
        var custom = false
        compose.setContent {
            MailcraneTheme { SnoozeChoices(now = now, onPick = { picked = it }, onCustom = { custom = true }) }
        }
        compose.onNodeWithText("Later today").assertIsDisplayed()
        compose.onNodeWithText("Tomorrow").assertIsDisplayed()
        compose.onNodeWithText("This weekend").assertIsDisplayed()
        compose.onNodeWithText("Next week").assertIsDisplayed()
        compose.onNodeWithText("Sat 28 Nov", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Tomorrow").performClick()
        assertEquals(snoozeTime(SnoozePreset.TOMORROW, now).toEpochSecond(), picked)
        compose.onNodeWithText("Pick date and time").performClick()
        assertTrue(custom)
    }

    @Test
    fun bannerSaysWhenAndOffersUnsnooze() {
        var unsnoozed = false
        val info = SnoozeInfo(wakeAt = now.plusDays(1).toEpochSecond(), deviceOnly = true, waking = false)
        compose.setContent { MailcraneTheme { SnoozedBanner(info, onUnsnooze = { unsnoozed = true }) } }
        compose.onNodeWithText("Snoozed until", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Only this device", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Unsnooze").performClick()
        assertTrue(unsnoozed)
    }

    @Test
    fun bannerHidesTheDeviceNoteWhenTheServerKnows() {
        val info = SnoozeInfo(wakeAt = now.plusDays(1).toEpochSecond(), deviceOnly = false, waking = false)
        compose.setContent { MailcraneTheme { SnoozedBanner(info, onUnsnooze = {}) } }
        compose.onNodeWithText("Only this device", substring = true).assertDoesNotExist()
    }
}
