package app.mailcrane.ui.compose

import app.mailcrane.core.RichBlock
import app.mailcrane.core.RichSpan
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RichTextTest {
    private val b = CharStyle(bold = true)

    @Test
    fun stylesFollowEdits() {
        var styles = toggleFormat("hello world", emptyList(), 0, 5, InlineFormat.BOLD)
        assertEquals(List(5) { b } + List(6) { PLAIN_STYLE }, styles)
        // Typing right after bold text stays bold; typing elsewhere doesn't.
        styles = restyle("hello world", "hello! world", styles)
        assertEquals(b, styles[5])
        styles = restyle("hello! world", "Xhello! world", styles)
        assertEquals(PLAIN_STYLE, styles[0])
        assertEquals(b, styles[1])
        // Deleting keeps the rest aligned.
        styles = restyle("Xhello! world", "Xhello", styles)
        assertEquals(6, styles.size)
        assertTrue(styles.drop(1).all { it.bold })
    }

    @Test
    fun formatTogglesOffWhenAllHaveIt() {
        val on = toggleFormat("abc", emptyList(), 0, 3, InlineFormat.ITALIC)
        assertTrue(on.all { it.italic })
        val off = toggleFormat("abc", on, 0, 3, InlineFormat.ITALIC)
        assertTrue(off.none { it.italic })
        assertEquals("an empty selection changes nothing", on, toggleFormat("abc", on, 1, 1, InlineFormat.ITALIC))
    }

    @Test
    fun listsAreAddedRemovedAndNumbered() {
        val text = "one\ntwo\nthree"
        val numbered = toggleList(text, emptyList(), 0, text.length, ListKind.NUMBERED)
        assertEquals("1. one\n2. two\n3. three", numbered.text)
        assertEquals(numbered.text.length, numbered.styles.size)
        val plain = toggleList(numbered.text, numbered.styles, 0, numbered.text.length, ListKind.NUMBERED)
        assertEquals(text, plain.text)
        val bullets = toggleList(text, emptyList(), 4, 5, ListKind.BULLET)
        assertEquals("one\n• two\nthree", bullets.text)
        assertEquals(6, bullets.selectionStart)
    }

    @Test
    fun enterContinuesALIst() {
        assertEquals("• a\n• " to 6, continueList("• a", "• a\n", 4))
        assertEquals("1. a\n2. " to 8, continueList("1. a", "1. a\n", 5))
        assertNull(continueList("plain", "plain\n", 6))
    }

    @Test
    fun theDocumentHasParagraphsListsAndLinks() {
        val text = "Hi there\n• one\n• two\n\n1. first\nBye"
        var styles = toggleFormat(text, emptyList(), 0, 2, InlineFormat.BOLD)
        styles = setLink(text, styles, 3, 8, "https://example.org")
        assertTrue(hasFormatting(text, styles))
        val doc = toRichDoc(text, styles)
        assertEquals(
            listOf(
                RichBlock.Paragraph(
                    listOf(
                        RichSpan("Hi", true, false, null),
                        RichSpan(" ", false, false, null),
                        RichSpan("there", false, false, "https://example.org"),
                    ),
                ),
                RichBlock.BulletList(
                    listOf(listOf(RichSpan("one", false, false, null)), listOf(RichSpan("two", false, false, null))),
                ),
                RichBlock.NumberedList(listOf(listOf(RichSpan("first", false, false, null)))),
                RichBlock.Paragraph(listOf(RichSpan("Bye", false, false, null))),
            ),
            doc.blocks,
        )
        assertFalse(hasFormatting("just text", emptyList()))
    }
}
