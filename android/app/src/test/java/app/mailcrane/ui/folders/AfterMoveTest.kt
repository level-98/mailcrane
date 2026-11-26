package app.mailcrane.ui.folders

import app.mailcrane.settings.AfterMoveAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** M4-15: which message shows after the open one is archived, deleted or moved. */
class AfterMoveTest {
    private val order = listOf(10L, 20L, 30L)

    @Test
    fun backToTheListIsTheDefaultAndNeverOpensAnything() {
        assertNull(messageAfterLeaving(order, 20, 1, AfterMoveAction.LIST))
    }

    @Test
    fun nextAndPreviousFollowTheListWhileItStillHoldsTheMessage() {
        // One pane: the list isn't on screen, so its last order still has the message.
        assertEquals(30L, messageAfterLeaving(order, 20, 1, AfterMoveAction.NEXT))
        assertEquals(10L, messageAfterLeaving(order, 20, 1, AfterMoveAction.PREVIOUS))
    }

    @Test
    fun aListThatAlreadyDroppedTheMessageStillGivesItsNeighbours() {
        // Side by side: the list refreshed first, so 30 moved up into 20's place.
        val after = listOf(10L, 30L)
        assertEquals(30L, messageAfterLeaving(after, 20, 1, AfterMoveAction.NEXT))
        assertEquals(10L, messageAfterLeaving(after, 20, 1, AfterMoveAction.PREVIOUS))
    }

    @Test
    fun theEndsOfTheListGoBackToTheList() {
        assertNull(messageAfterLeaving(order, 30, 2, AfterMoveAction.NEXT))
        assertNull(messageAfterLeaving(order, 10, 0, AfterMoveAction.PREVIOUS))
        assertNull(messageAfterLeaving(listOf(10L, 20L), 30, 2, AfterMoveAction.NEXT))
        assertNull(messageAfterLeaving(listOf(20L, 30L), 10, 0, AfterMoveAction.PREVIOUS))
        assertNull(messageAfterLeaving(emptyList(), 10, 0, AfterMoveAction.NEXT))
    }

    @Test
    fun aMessageOpenedFromElsewhereGoesBackToTheList() {
        // Opened from search: not in the folder's list when it opened.
        assertNull(messageAfterLeaving(order, 99, -1, AfterMoveAction.NEXT))
        assertNull(messageAfterLeaving(order, 99, -1, AfterMoveAction.PREVIOUS))
    }
}
