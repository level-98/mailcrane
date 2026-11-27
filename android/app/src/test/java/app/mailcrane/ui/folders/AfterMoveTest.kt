package app.mailcrane.ui.folders

import app.mailcrane.settings.AfterMoveAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** M4-15: which message shows after the open one is archived, deleted or moved. */
class AfterMoveTest {
    private val order = listOf(10L, 20L, 30L)
    private val of20 = neighboursIn(order, 20)

    @Test
    fun backToTheListIsTheDefaultAndNeverOpensAnything() {
        assertNull(messageAfterLeaving(of20, AfterMoveAction.LIST, order))
    }

    @Test
    fun nextAndPreviousAreTheNeighboursFromWhenItOpened() {
        assertEquals(Neighbours(previous = 10, next = 30), of20)
        // One pane: the list isn't on screen, so its last order still has the message.
        assertEquals(30L, messageAfterLeaving(of20, AfterMoveAction.NEXT, order))
        assertEquals(10L, messageAfterLeaving(of20, AfterMoveAction.PREVIOUS, order))
        // Side by side: the list refreshed first and dropped it.
        assertEquals(30L, messageAfterLeaving(of20, AfterMoveAction.NEXT, listOf(10L, 30L)))
    }

    @Test
    fun mailArrivingAboveDoesNotShiftThem() {
        // #372 review: new mail at the top moved every index; the neighbours are ids.
        val refreshed = listOf(5L, 10L, 30L)
        assertEquals(30L, messageAfterLeaving(of20, AfterMoveAction.NEXT, refreshed))
        assertEquals(10L, messageAfterLeaving(of20, AfterMoveAction.PREVIOUS, refreshed))
    }

    @Test
    fun aNeighbourThatIsGoneToo_orTheEnds_goBackToTheList() {
        assertNull(messageAfterLeaving(of20, AfterMoveAction.NEXT, listOf(10L)))
        assertNull(messageAfterLeaving(neighboursIn(order, 30), AfterMoveAction.NEXT, order))
        assertNull(messageAfterLeaving(neighboursIn(order, 10), AfterMoveAction.PREVIOUS, order))
    }

    @Test
    fun aMessageOpenedFromElsewhereGoesBackToTheList() {
        // Opened from search (no neighbours), or not in the list at all.
        assertNull(neighboursIn(order, 99))
        assertNull(neighboursIn(order, null))
        assertNull(messageAfterLeaving(null, AfterMoveAction.NEXT, order))
        assertNull(messageAfterLeaving(null, AfterMoveAction.PREVIOUS, order))
    }

    @Test
    fun aNeighbourGoneFromTheListIsNotOpenedEvenWithNothingShown() {
        // #514: the current list comes from the store now; empty means it holds nothing, not "unknown".
        assertNull(messageAfterLeaving(of20, AfterMoveAction.NEXT, emptyList()))
    }

    @Test
    fun theOriginIsTheListItWasOpenedFrom() {
        // #514: from search there is none, so no neighbours; from a conversation it is that thread.
        assertNull(originOf(20, openedFromSearch = true, folderId = 1, threadId = null, order = order))
        assertNull(originOf(null, openedFromSearch = false, folderId = 1, threadId = null, order = order))
        assertEquals(Origin(1, null, order), originOf(20, false, 1, null, order))
        val thread = listOf(5L, 20L, 6L)
        val fromThread = originOf(20, false, 1, 9, thread)
        assertEquals(Origin(1, 9, thread), fromThread)
        assertEquals(Neighbours(previous = 5, next = 6), neighboursIn(fromThread!!.order, 20))
    }
}
