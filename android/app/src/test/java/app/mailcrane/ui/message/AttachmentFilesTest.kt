package app.mailcrane.ui.message

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AttachmentFilesTest {
    @Test
    fun riskyTypesAskFirst() {
        for (name in listOf("app.apk", "SETUP.EXE", "page.html", "logo.svg", "run.js", "a.b.bat")) {
            assertTrue(name, isRiskyAttachment(name, "application/octet-stream"))
        }
        assertTrue(isRiskyAttachment("x", "application/vnd.android.package-archive"))
        assertTrue(isRiskyAttachment("x", "text/html"))
        for (name in listOf("photo.jpg", "report.pdf", "notes.txt", "noextension")) {
            assertFalse(name, isRiskyAttachment(name, "application/pdf"))
        }
    }
}
