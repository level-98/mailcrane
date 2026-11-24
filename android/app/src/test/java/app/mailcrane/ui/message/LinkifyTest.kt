package app.mailcrane.ui.message

import androidx.compose.ui.text.LinkAnnotation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.ZoneOffset
import java.util.Locale

class LinkifyTest {
    private fun links(text: String): List<Pair<String, String>> {
        val annotated = linkify(text) {}
        return annotated.getLinkAnnotations(0, annotated.length).map {
            val url = (it.item as LinkAnnotation.Clickable).tag
            text.substring(it.start, it.end) to url
        }
    }

    @Test
    fun finds_web_links_and_addresses() {
        assertEquals(
            listOf(
                "https://example.org/a?b=1" to "https://example.org/a?b=1",
                "www.example.com" to "https://www.example.com",
                "ann@example.net" to "mailto:ann@example.net",
            ),
            links("See https://example.org/a?b=1, www.example.com or write to ann@example.net."),
        )
    }

    @Test
    fun trailing_punctuation_is_not_part_of_the_link() {
        assertEquals(listOf("http://example.org/x" to "http://example.org/x"), links("(http://example.org/x)."))
    }

    @Test
    fun ordinary_words_and_other_schemes_are_not_links() {
        assertTrue(links("e.g. version 1.2 at example dot org").isEmpty())
        assertTrue(links("javascript:alert(1) file:///etc/passwd").isEmpty())
    }

    @Test
    fun the_text_is_unchanged() {
        val text = "Hello https://example.org\nBye"
        assertEquals(text, linkify(text) {}.text)
    }

    @Test
    fun full_date_handles_any_value() {
        assertTrue(formatFullDate(1_772_445_600, ZoneOffset.UTC, Locale.UK).contains("2026"))
        assertEquals("", formatFullDate(Long.MAX_VALUE, ZoneOffset.UTC, Locale.UK))
        assertEquals("", formatFullDate(Long.MIN_VALUE, ZoneOffset.UTC, Locale.UK))
    }
}
