package app.mailcrane.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SenderAvatarTest {
    @Test
    fun latinNamesGiveFirstAndLastInitials() {
        assertEquals("AL", avatarInitials("Ana López", "ana@example.org"))
        assertEquals("AL", avatarInitials("Ana María López", "ana@example.org"))
        assertEquals("A", avatarInitials("ana", "ana@example.org"))
        assertEquals("ÉD", avatarInitials("élodie dupont", "e@example.org"))
        assertEquals("LA", avatarInitials("\"López, Ana\"", "ana@example.org"))
    }

    @Test
    fun aDisplayNameThatIsTheSendersDomainGivesOneInitial() {
        // #389: these gave EC and EO, from splitting on the dots.
        assertEquals("E", avatarInitials("Example.com", "news@example.com"))
        assertEquals("E", avatarInitials("example.org", "no-reply@example.org"))
        // A parent of the address's domain counts too, compared case-insensitively.
        assertEquals("E", avatarInitials("EXAMPLE.NET", "news@mail.example.net"))
        // Brackets, punctuation and a leading "www." around it are ignored.
        assertEquals("E", avatarInitials("(Example.com)", "news@example.com"))
        assertEquals("E", avatarInitials("Example.com.", "news@example.com"))
        assertEquals("E", avatarInitials("[example.org]", "news@example.org"))
        assertEquals("E", avatarInitials("www.example.com", "news@example.com"))
    }

    @Test
    fun anyOtherDottedNameKeepsBothInitials() {
        // The name has to match the address: dotted personal names keep both initials.
        assertEquals("JD", avatarInitials("Jane.Doe", "jane@example.org"))
        assertEquals("JS", avatarInitials("John.Smith", "js@example.com"))
        assertEquals("AL", avatarInitials("Ana.Lopez", "ana@example.net"))
        // A domain that isn't the sender's, or only part of a label of it, splits as before.
        assertEquals("EO", avatarInitials("Example.org", "news@example.net"))
        assertEquals("PC", avatarInitials("ple.com", "news@example.com"))
        // Names with spaces keep their first and last initials, dots or not.
        assertEquals("JD", avatarInitials("Jane Doe", "jane@example.org"))
        assertEquals("JD", avatarInitials("Jane Q. Doe", "jane@example.org"))
        assertEquals("V", avatarInitials("v.2", "v@example.org"))
        assertEquals("A", avatarInitials("Ana", "ana@example.org"))
    }

    @Test
    fun aNoBreakSpaceSeparatesWordsToo() {
        // After RFC 2047 decoding a name can hold U+00A0 between its words.
        assertEquals("AL", avatarInitials("Ana\u00A0López", "ana@example.org"))
    }

    @Test
    fun anAddressLocalPartKeepsBothInitials() {
        // Not a display name: "ana.lopez" holds no domain, so it still splits on the dot.
        assertEquals("AL", avatarInitials("", "ana.lopez@example.org"))
        assertEquals("JD", avatarInitials("", "jane.doe@example.org"))
    }

    @Test
    fun otherScriptsGiveTheirFirstGrapheme() {
        assertEquals("山", avatarInitials("山田 太郎", "taro@example.org"))
        assertEquals("김", avatarInitials("김민준", "min@example.org"))
        assertEquals("А", avatarInitials("Анна Петрова", "anna@example.org"))
        assertEquals("م", avatarInitials("محمد علي", "m@example.org"))
    }

    @Test
    fun withoutANameTheAddressGivesTheInitials() {
        assertEquals("AL", avatarInitials("", "ana.lopez@example.org"))
        assertEquals("B", avatarInitials("  ", "bob@example.org"))
        // A display name that is really an address counts as one.
        assertEquals("NR", avatarInitials("no-reply@example.org", "no-reply@example.org"))
    }

    @Test
    fun emojiSymbolsAndNothingGiveThePersonIcon() {
        assertNull(avatarInitials("🎉 Party", "party@example.org"))
        assertNull(avatarInitials("★", ""))
        assertNull(avatarInitials("", ""))
        assertNull(avatarInitials("", "@example.org"))
    }

    @Test
    fun theColourIsStableAndIgnoresCase() {
        val index = avatarColorIndex("ana@example.org")
        assertEquals(index, avatarColorIndex("ANA@Example.org"))
        assertEquals(index, avatarColorIndex("  ana@example.org "))
        // Pinned, so a change to the hash (which would recolour everyone) is noticed.
        assertEquals(PINNED_INDEX, index)
        val spread = (1..200).map { avatarColorIndex("user$it@example.org") }.toSet()
        assertEquals("every colour gets used", AVATAR_PALETTE.size, spread.size)
        assertTrue(spread.all { it in AVATAR_PALETTE.indices })
    }

    @Test
    fun everyColourMeetsWcagAaInLightAndDark() {
        for (c in AVATAR_PALETTE) {
            assertTrue("light ${c.lightBackground}", contrast(c.lightBackground, c.lightText) >= 4.5)
            assertTrue("dark ${c.darkBackground}", contrast(c.darkBackground, c.darkText) >= 4.5)
        }
    }

    private fun contrast(
        a: Color,
        b: Color,
    ): Double {
        val la = a.luminance().toDouble()
        val lb = b.luminance().toDouble()
        return (maxOf(la, lb) + 0.05) / (minOf(la, lb) + 0.05)
    }

    private companion object {
        /** FNV-1a 32 of "ana@example.org" is 1184264263; mod 8 is 7 (worked out independently in Python). */
        const val PINNED_INDEX = 7
    }
}
