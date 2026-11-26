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
    fun aDisplayNameThatIsADomainGivesOneInitial() {
        // #389: these gave AU, BC and EU, from splitting on the dots.
        assertEquals("A", avatarInitials("Amazon.co.uk", "no-reply@example.co.uk"))
        assertEquals("B", avatarInitials("Booking.com", "no-reply@example.com"))
        assertEquals("E", avatarInitials("eBay.co.uk", "ebay@example.co.uk"))
        assertEquals("E", avatarInitials("Example.org", "info@example.org"))
        // The rule: no whitespace, and a dot before a last label of two or more letters. It can't tell a surname
        // from a top-level domain, so a dotted display name such as "jane.doe" gives one initial too.
        assertEquals("J", avatarInitials("jane.doe", "jane@example.org"))
        // Names with spaces keep their first and last initials, dots or not.
        assertEquals("JD", avatarInitials("Jane Doe", "jane@example.org"))
        assertEquals("JD", avatarInitials("Jane Q. Doe", "jane@example.org"))
        // A short or non-letter last label isn't a domain: still split.
        assertEquals("AB", avatarInitials("A.B", "ab@example.org"))
        // "v.2" is split too, and a last word that starts with no letter adds no initial (as before).
        assertEquals("V", avatarInitials("v.2", "v@example.org"))
        // Single words are unchanged.
        assertEquals("A", avatarInitials("Ana", "ana@example.org"))
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
