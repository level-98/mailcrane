package app.mailcrane.settings

import app.mailcrane.core.RuleActionInfo
import app.mailcrane.core.RuleConditionInfo
import app.mailcrane.core.RuleInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class RuleDraftTest {
    @Test
    fun aCompleteDraftBecomesARule() {
        val draft =
            RuleDraft(
                name = " Newsletters ",
                matchAll = false,
                conditions =
                    listOf(
                        ConditionDraft(ConditionKind.FROM, " news@shop.example "),
                        ConditionDraft(ConditionKind.HAS_ATTACHMENT),
                        ConditionDraft(ConditionKind.OLDER_THAN, "30"),
                    ),
                actions = listOf(ActionDraft(ActionKind.MARK_READ), ActionDraft(ActionKind.MOVE, 9)),
            )
        val input = draft.toInput(accountId = 3)!!
        assertEquals("Newsletters", input.name)
        assertEquals(3L, input.accountId)
        assertEquals(false, input.matchAll)
        assertEquals(
            listOf(
                RuleConditionInfo.From("news@shop.example"),
                RuleConditionInfo.HasAttachment,
                RuleConditionInfo.OlderThanDays(30u),
            ),
            input.conditions,
        )
        assertEquals(listOf(RuleActionInfo.MarkRead, RuleActionInfo.Move(9)), input.actions)
        // And back.
        assertEquals(
            draft.copy(
                id = 5,
                name = "Newsletters",
                conditions =
                    draft.conditions.map {
                        it.copy(text = it.text.trim())
                    },
            ),
            RuleDraft.of(RuleInfo(5, input)),
        )
    }

    @Test
    fun incompleteDraftsAreRefused() {
        assertNull(RuleDraft(name = "").toInput(1))
        assertNull(RuleDraft(name = "x", conditions = listOf(ConditionDraft(ConditionKind.SUBJECT, " "))).toInput(1))
        assertNull(
            RuleDraft(name = "x", conditions = listOf(ConditionDraft(ConditionKind.OLDER_THAN, "soon"))).toInput(1),
        )
        assertNull(
            RuleDraft(
                name = "x",
                conditions = listOf(ConditionDraft(ConditionKind.SUBJECT, "a")),
                actions = listOf(ActionDraft(ActionKind.MOVE)),
            ).toInput(1),
        )
        assertNull(RuleDraft(name = "x", conditions = emptyList()).toInput(1))
    }

    @Test
    fun fromAMessageItsSenderIsTheCondition() {
        val d = RuleDraft.fromSender("news@shop.example")
        assertEquals(listOf(ConditionDraft(ConditionKind.FROM, "news@shop.example")), d.conditions)
        assertNull("a folder still has to be picked", d.toInput(1))
        assertEquals(RuleDraft(name = "x").thisAccountOnly, true)
        assertEquals(null, RuleDraft(name = "x", thisAccountOnly = false).toInput(1)?.accountId)
    }
}
