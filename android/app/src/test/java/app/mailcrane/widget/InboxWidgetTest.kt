package app.mailcrane.widget

import app.mailcrane.core.WidgetMessage
import app.mailcrane.core.WidgetSnapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class InboxWidgetTest {
    private val snapshot =
        WidgetSnapshot(
            3u,
            listOf(WidgetMessage(1, 2, 2u, "Ana", "Lunch", 100, false)),
        )

    @Test
    fun contentIsShownNormally() {
        val c = widgetContent(snapshot, hideContent = false)
        assertEquals(3L, c.unread)
        assertEquals(listOf("Lunch"), c.messages.map { it.subject })
    }

    @Test
    fun hidingContentKeepsOnlyTheCount() {
        val c = widgetContent(snapshot, hideContent = true)
        assertEquals(3L, c.unread)
        assertTrue("no senders or subjects", c.messages.isEmpty())
    }
}
