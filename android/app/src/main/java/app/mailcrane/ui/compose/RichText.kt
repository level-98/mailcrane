package app.mailcrane.ui.compose

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import app.mailcrane.core.RichBlock
import app.mailcrane.core.RichDoc
import app.mailcrane.core.RichSpan
import java.io.Serializable

/**
 * Rich-text compose (M2-09) without an editor library: the body stays plain text, with one [CharStyle] per
 * character for bold, italic and links. Lists are the text prefixes [BULLET] and `1. ` at the start of a
 * line, which the toolbar adds and removes. [toRichDoc] turns both into the core's [RichDoc].
 */
data class CharStyle(
    val bold: Boolean = false,
    val italic: Boolean = false,
    val link: String? = null,
) : Serializable

val PLAIN_STYLE = CharStyle()

/** The bullet a bulleted line starts with. */
const val BULLET = "• "

private val NUMBERED = Regex("^(\\d{1,3})\\. ")

/** [styles] made to fit [text]: extra entries dropped, missing ones plain. */
fun fitStyles(
    text: String,
    styles: List<CharStyle>,
): List<CharStyle> =
    when {
        styles.size == text.length -> styles
        styles.size > text.length -> styles.take(text.length)
        else -> styles + List(text.length - styles.size) { PLAIN_STYLE }
    }

/**
 * The styles after the text changed from [old] to [new]: the unchanged start and end keep theirs, and
 * typed characters take the style of the character before them (so typing after bold text stays bold).
 */
fun restyle(
    old: String,
    new: String,
    styles: List<CharStyle>,
): List<CharStyle> {
    val fitted = fitStyles(old, styles)
    var prefix = 0
    val max = minOf(old.length, new.length)
    while (prefix < max && old[prefix] == new[prefix]) prefix++
    var suffix = 0
    while (suffix < max - prefix && old[old.length - 1 - suffix] == new[new.length - 1 - suffix]) suffix++
    val inserted = new.length - prefix - suffix
    // A new line starts plain; within a line, typing continues the style before it.
    val carry = fitted.getOrNull(prefix - 1)?.takeIf { prefix > 0 && old[prefix - 1] != '\n' } ?: PLAIN_STYLE
    val insertedStyles =
        List(inserted.coerceAtLeast(0)) { i ->
            if (new[prefix + i] == '\n') PLAIN_STYLE else carry.copy(link = null)
        }
    return fitted.subList(0, prefix) + insertedStyles + fitted.subList(old.length - suffix, old.length)
}

/** Which inline format a toolbar button changes. */
enum class InlineFormat { BOLD, ITALIC }

/**
 * Turns [format] on for the selected characters, or off if they all have it already. An empty selection
 * changes nothing.
 */
fun toggleFormat(
    text: String,
    styles: List<CharStyle>,
    start: Int,
    end: Int,
    format: InlineFormat,
): List<CharStyle> {
    val fitted = fitStyles(text, styles)
    val (s, e) = minOf(start, end).coerceIn(0, text.length) to maxOf(start, end).coerceIn(0, text.length)
    if (s == e) return fitted
    val has: (CharStyle) -> Boolean = { if (format == InlineFormat.BOLD) it.bold else it.italic }
    val turnOn = !fitted.subList(s, e).all(has)
    return fitted.mapIndexed { i, st ->
        if (i !in s until e) {
            st
        } else if (format == InlineFormat.BOLD) {
            st.copy(bold = turnOn)
        } else {
            st.copy(italic = turnOn)
        }
    }
}

/** Sets (or with null removes) a link on the selected characters. */
fun setLink(
    text: String,
    styles: List<CharStyle>,
    start: Int,
    end: Int,
    url: String?,
): List<CharStyle> {
    val fitted = fitStyles(text, styles)
    val (s, e) = minOf(start, end).coerceIn(0, text.length) to maxOf(start, end).coerceIn(0, text.length)
    return fitted.mapIndexed { i, st -> if (i in s until e) st.copy(link = url?.trim()?.ifEmpty { null }) else st }
}

/** Which list a toolbar button makes. */
enum class ListKind { BULLET, NUMBERED }

/** Text and styles after a list change, and where the selection moves. */
data class ListEdit(
    val text: String,
    val styles: List<CharStyle>,
    val selectionStart: Int,
    val selectionEnd: Int,
)

/**
 * Makes the lines touched by the selection a [kind] list, or plain lines if they all are one already.
 * Numbered lines are numbered 1, 2, 3… in each run.
 */
fun toggleList(
    text: String,
    styles: List<CharStyle>,
    start: Int,
    end: Int,
    kind: ListKind,
): ListEdit {
    val fitted = fitStyles(text, styles)
    val lines = text.split('\n')
    val lineStarts = lines.runningFold(0) { acc, l -> acc + l.length + 1 }
    val first = lineStarts.indexOfLast { it <= minOf(start, end) }.coerceAtLeast(0)
    val last = lineStarts.indexOfLast { it <= maxOf(start, end) }.coerceIn(first, lines.lastIndex)
    val isKind: (String) -> Boolean = {
        if (kind ==
            ListKind.BULLET
        ) {
            it.startsWith(BULLET)
        } else {
            NUMBERED.containsMatchIn(it)
        }
    }
    val turnOn = !(first..last).all { isKind(lines[it]) }
    val outText = StringBuilder()
    val outStyles = ArrayList<CharStyle>(fitted.size + 8)
    var number = 0
    var shiftBeforeSelection = 0
    var shiftTotal = 0
    for ((i, line) in lines.withIndex()) {
        if (i > 0) {
            outText.append('\n')
            outStyles.add(PLAIN_STYLE)
        }
        val lineStyles = fitted.subList(lineStarts[i], lineStarts[i] + line.length)
        val (body, bodyStyles) = stripListPrefix(line, lineStyles)
        val removed = line.length - body.length
        val prefix =
            if (i in first..last) {
                if (!turnOn) {
                    ""
                } else if (kind == ListKind.BULLET) {
                    BULLET
                } else {
                    "${++number}. "
                }
            } else {
                // Lines outside the selection stay as they are.
                line.substring(0, removed)
            }
        outText.append(prefix).append(body)
        repeat(prefix.length) { outStyles.add(PLAIN_STYLE) }
        outStyles.addAll(bodyStyles)
        val delta = prefix.length - removed
        if (i == first) shiftBeforeSelection = delta
        if (i in first..last) shiftTotal += delta
    }
    return ListEdit(
        text = outText.toString(),
        styles = outStyles,
        selectionStart = (minOf(start, end) + shiftBeforeSelection).coerceIn(0, outText.length),
        selectionEnd = (maxOf(start, end) + shiftTotal).coerceIn(0, outText.length),
    )
}

private fun stripListPrefix(
    line: String,
    styles: List<CharStyle>,
): Pair<String, List<CharStyle>> {
    val n =
        when {
            line.startsWith(BULLET) -> BULLET.length
            else -> NUMBERED.find(line)?.value?.length ?: 0
        }
    return line.substring(n) to styles.subList(n, styles.size)
}

/** After Enter at the end of a list line, the new line continues the list. */
fun continueList(
    old: String,
    new: String,
    cursor: Int,
): Pair<String, Int>? {
    if (new.length != old.length + 1 || cursor <= 0 || new.getOrNull(cursor - 1) != '\n') return null
    val prevStart = new.lastIndexOf('\n', cursor - 2) + 1
    val prev = new.substring(prevStart, cursor - 1)
    val prefix =
        when {
            prev == BULLET.trimEnd() || prev == BULLET -> return null
            prev.startsWith(BULLET) -> BULLET
            else ->
                NUMBERED
                    .find(prev)
                    ?.groupValues
                    ?.get(1)
                    ?.toIntOrNull()
                    ?.let { "${it + 1}. " } ?: return null
        }
    return (new.substring(0, cursor) + prefix + new.substring(cursor)) to (cursor + prefix.length)
}

/** Whether there is any formatting or list, i.e. whether sending as rich text is worth it. */
fun hasFormatting(
    text: String,
    styles: List<CharStyle>,
): Boolean =
    fitStyles(text, styles).any { it != PLAIN_STYLE } ||
        text.split('\n').any { it.startsWith(BULLET) || NUMBERED.containsMatchIn(it) }

private fun spansOf(
    text: String,
    styles: List<CharStyle>,
): List<RichSpan> {
    val out = mutableListOf<RichSpan>()
    var i = 0
    while (i < text.length) {
        val st = styles[i]
        var j = i + 1
        while (j < text.length && styles[j] == st) j++
        out += RichSpan(text.substring(i, j), st.bold, st.italic, st.link)
        i = j
    }
    return out
}

/** The editor's content as the core's [RichDoc]. Each other line is a paragraph; blank lines are dropped. */
fun toRichDoc(
    text: String,
    styles: List<CharStyle>,
): RichDoc {
    val fitted = fitStyles(text, styles)
    val blocks = mutableListOf<RichBlock>()
    var offset = 0
    var bullets = mutableListOf<List<RichSpan>>()
    var numbers = mutableListOf<List<RichSpan>>()

    fun flush() {
        if (bullets.isNotEmpty()) blocks += RichBlock.BulletList(bullets)
        if (numbers.isNotEmpty()) blocks += RichBlock.NumberedList(numbers)
        bullets = mutableListOf()
        numbers = mutableListOf()
    }
    for (line in text.split('\n')) {
        val lineStyles = fitted.subList(offset, offset + line.length)
        offset += line.length + 1
        val (body, bodyStyles) = stripListPrefix(line, lineStyles)
        when {
            line.startsWith(BULLET) -> {
                if (numbers.isNotEmpty()) flush()
                bullets += spansOf(body, bodyStyles)
            }
            NUMBERED.containsMatchIn(line) -> {
                if (bullets.isNotEmpty()) flush()
                numbers += spansOf(body, bodyStyles)
            }
            else -> {
                flush()
                if (line.isNotBlank()) blocks += RichBlock.Paragraph(spansOf(line, lineStyles))
            }
        }
    }
    flush()
    return RichDoc(blocks)
}

/** How the editor draws the styles (links underlined in [linkColor]). */
fun styledText(
    text: String,
    styles: List<CharStyle>,
    linkColor: Color,
): AnnotatedString {
    val fitted = fitStyles(text, styles)
    return buildAnnotatedString {
        append(text)
        var i = 0
        while (i < text.length) {
            val st = fitted[i]
            var j = i + 1
            while (j < text.length && fitted[j] == st) j++
            if (st != PLAIN_STYLE) {
                addStyle(
                    SpanStyle(
                        fontWeight = if (st.bold) FontWeight.Bold else null,
                        fontStyle = if (st.italic) FontStyle.Italic else null,
                        color = if (st.link != null) linkColor else Color.Unspecified,
                        textDecoration = if (st.link != null) TextDecoration.Underline else null,
                    ),
                    i,
                    j,
                )
            }
            i = j
        }
    }
}
