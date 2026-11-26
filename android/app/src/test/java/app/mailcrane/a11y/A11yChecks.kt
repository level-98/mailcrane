package app.mailcrane.a11y

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp

/** The smallest touch target allowed, per Material and WCAG 2.2 target size guidance (M6-01). */
val MIN_TOUCH = 48.dp

/** What a node says to TalkBack, or null when it says nothing. */
private fun SemanticsNode.label(): String? {
    val c = config
    val parts =
        listOfNotNull(
            c.getOrNull(SemanticsProperties.ContentDescription)?.joinToString(" "),
            c.getOrNull(SemanticsProperties.Text)?.joinToString(" ") { it.text },
            c.getOrNull(SemanticsProperties.EditableText)?.text,
            c.getOrNull(SemanticsProperties.StateDescription),
        ).filter { it.isNotBlank() }
    return parts.joinToString(" ").ifBlank { null }
}

private fun SemanticsNode.describe(): String {
    val tag = config.getOrNull(SemanticsProperties.TestTag)
    val b = boundsInRoot
    return (tag?.let { "tag=$it " } ?: "") + "label=${label()} at (${b.left},${b.top} ${b.width}x${b.height})"
}

private fun SemanticsNode.all(): List<SemanticsNode> = listOf(this) + children.flatMap { it.all() }

/**
 * Checks the screen as TalkBack sees it (the merged semantics tree) and fails with every problem at
 * once:
 * - every clickable control has a label (text, content description or state);
 * - every clickable control's touch area is at least [MIN_TOUCH] in both directions;
 * - there is at least one heading, unless [needsHeading] is false;
 * - no text is cut off vertically (its layout overflows its height), which large font sizes cause.
 */
fun ComposeContentTestRule.assertAccessible(
    screen: String,
    needsHeading: Boolean = true,
) {
    val root = onRoot().fetchSemanticsNode()
    val density: Density = this.density
    val minPx = with(density) { MIN_TOUCH.toPx() }
    val problems = mutableListOf<String>()
    val nodes = root.all()
    for (node in nodes) {
        val c = node.config
        val clickable = c.contains(SemanticsActions.OnClick) || c.contains(SemanticsActions.OnLongClick)
        if (clickable) {
            if (node.label() == null) problems += "unlabelled control: ${node.describe()}"
            // The touch area: the layout size (unclipped, so a control scrolled out of view still
            // counts) or the extended touch bounds, whichever is larger.
            val t = node.touchBoundsInRoot
            val w = maxOf(t.width, node.size.width.toFloat())
            val h = maxOf(t.height, node.size.height.toFloat())
            // Half a pixel of slack for rounding.
            if (w + 0.5f < minPx || h + 0.5f < minPx) {
                problems += "touch target under 48dp (${w}x${h}px): ${node.describe()}"
            }
        }
        val layouts = mutableListOf<TextLayoutResult>()
        c.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(layouts)
        if (layouts.any { it.didOverflowHeight }) problems += "text cut off: ${node.describe()}"
    }
    if (needsHeading && nodes.none { it.config.contains(SemanticsProperties.Heading) }) {
        problems += "no heading on the screen"
    }
    if (problems.isNotEmpty()) {
        throw AssertionError("$screen has accessibility problems:\n" + problems.joinToString("\n") { "  - $it" })
    }
}
