package app.mailcrane.settings

import android.app.Application
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import app.mailcrane.ui.theme.MailcraneTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
// A plain Application: the real one loads the native core, which these tests don't need.
@Config(sdk = [35], application = Application::class)
class LicencesScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val sample =
        """
        {"texts":{"rust-0":{"id":"MIT","text":"Copyright (c) Example\nPermission is hereby granted"},
                  "spdx-Apache-2.0":{"id":"Apache-2.0","text":"Apache License\nVersion 2.0"}},
         "libraries":[
           {"name":"androidx.core:core","version":"1.17.0","kind":"android","licence":"Apache-2.0","text":"spdx-Apache-2.0","url":""},
           {"name":"tokio","version":"1.48.0","kind":"rust","licence":"MIT","text":"rust-0","url":"https://example.org/tokio"}]}
        """.trimIndent()

    @Test
    fun parsesTheGeneratedFile() {
        val l = OssLicences.parse(sample)
        assertEquals(listOf("androidx.core:core", "tokio"), l.libraries.map { it.name })
        assertEquals("rust", l.libraries[1].kind)
        assertTrue(l.texts.getValue("rust-0").startsWith("Copyright (c) Example"))
    }

    @Test
    fun malformedFilesGiveAnEmptyList() {
        for (bad in listOf("", "{", "[]", """{"texts":{}}""", """{"texts":[],"libraries":[]}""")) {
            assertEquals(bad, OssLicences.EMPTY, OssLicences.parse(bad))
        }
    }

    @Test
    fun listsLibrariesAndOpensTheirLicence() {
        compose.setContent { MailcraneTheme { LicencesContent(OssLicences.parse(sample), onBack = {}) } }
        compose.onNodeWithText("Rust library 1.48.0 · MIT").assertIsDisplayed()
        compose.onNodeWithText("Android library 1.17.0 · Apache-2.0").assertIsDisplayed()
        compose.onNodeWithText("tokio").performClick()
        compose.onNodeWithText("Permission is hereby granted", substring = true).assertIsDisplayed()
    }

    // The build's own file: generated from the real dependencies, so it must parse and list both halves.
    @Test
    fun theBundledFileListsRustAndAndroidLibraries() {
        val l = OssLicences.load(ApplicationProvider.getApplicationContext())
        assertTrue("libraries: ${l.libraries.size}", l.libraries.size > 50)
        assertTrue(l.libraries.any { it.kind == "rust" && it.name == "tokio" })
        assertTrue(l.libraries.any { it.kind == "android" && it.name.startsWith("androidx.") })
        assertTrue(l.libraries.all { l.texts[it.textKey].orEmpty().isNotBlank() })
    }
}
