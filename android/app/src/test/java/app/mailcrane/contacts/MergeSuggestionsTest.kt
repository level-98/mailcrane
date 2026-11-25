package app.mailcrane.contacts

import org.junit.Assert.assertEquals
import org.junit.Test

class MergeSuggestionsTest {
    @Test
    fun correspondentsComeFirstAndDuplicatesGo() {
        val past =
            listOf(RecipientSuggestion("Ana Silva", "ana@example.net"), RecipientSuggestion("", "bob@example.net"))
        val device =
            listOf(
                RecipientSuggestion("Ana (phone)", "ANA@example.net", fromDevice = true),
                RecipientSuggestion("Bob Builder", "bob@example.net", fromDevice = true),
                RecipientSuggestion("Cy", "cy@example.net", fromDevice = true),
            )
        assertEquals(
            listOf(
                RecipientSuggestion("Ana Silva", "ana@example.net"),
                // No name in past mail: the contact's name is used.
                RecipientSuggestion("Bob Builder", "bob@example.net"),
                RecipientSuggestion("Cy", "cy@example.net", fromDevice = true),
            ),
            mergeSuggestions(past, device, 8),
        )
    }

    @Test
    fun theLimitHolds() {
        val many = (1..20).map { RecipientSuggestion("P$it", "p$it@example.net") }
        assertEquals(8, mergeSuggestions(many, emptyList(), 8).size)
        assertEquals(emptyList<RecipientSuggestion>(), mergeSuggestions(emptyList(), emptyList(), 8))
    }

    @Test
    fun withoutPermissionNoDeviceContactsAreRead() =
        kotlinx.coroutines.test.runTest {
            assertEquals(emptyList<RecipientSuggestion>(), NoDeviceContacts.search("ana", 8))
        }
}
