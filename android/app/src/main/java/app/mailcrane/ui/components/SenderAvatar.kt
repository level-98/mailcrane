package app.mailcrane.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.mailcrane.contacts.LocalContactPhotos
import app.mailcrane.ui.theme.LocalDarkTheme
import java.text.BreakIterator
import java.util.Locale

/**
 * A sender's picture in the message list (M4-24): a photo from the device's contacts when the user turned
 * that on, otherwise up to two initials on a colour that stays the same for the address. Names that give no
 * letters (emoji, symbols, nothing) show a generic person. Nothing here touches the network.
 *
 * Decorative for TalkBack: the row already reads the sender. While [selected] a check mark replaces it.
 * With [onClick] (selection in the message list) the whole 40 dp circle is the touch target.
 */
@Composable
fun SenderAvatar(
    name: String,
    address: String,
    modifier: Modifier = Modifier,
    selected: Boolean = false,
    onClick: (() -> Unit)? = null,
    size: Dp = AVATAR_SIZE,
) {
    val photos = LocalContactPhotos.current
    val photo by produceState<ImageBitmap?>(null, photos, address) {
        value = if (address.isBlank()) null else photos.photo(address)
    }
    val dark = LocalDarkTheme.current
    val initials = remember(name, address) { avatarInitials(name, address) }
    val colors = AVATAR_PALETTE[avatarColorIndex(address.ifBlank { name })]
    val background = if (dark) colors.darkBackground else colors.lightBackground
    val foreground = if (dark) colors.darkText else colors.lightText
    Box(
        modifier =
            modifier
                .size(size)
                .clip(CircleShape)
                .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
                .clearAndSetSemantics {},
        contentAlignment = Alignment.Center,
    ) {
        val current = photo
        when {
            selected -> CheckMark(MaterialTheme.colorScheme.primary, MaterialTheme.colorScheme.onPrimary)
            current != null ->
                Image(
                    bitmap = current,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.size(size),
                )
            initials != null ->
                Box(Modifier.size(size).background(background), contentAlignment = Alignment.Center) {
                    Text(
                        text = initials,
                        color = foreground,
                        // Fixed relative to the circle, which doesn't grow with font scale either.
                        fontSize = (size.value * 0.4f).sp,
                        fontWeight = FontWeight.Medium,
                        maxLines = 1,
                    )
                }
            else -> PersonGlyph(background, foreground)
        }
    }
}

/** The avatar's size: also its touch target when it selects the row. */
val AVATAR_SIZE = 40.dp

@Composable
private fun CheckMark(
    background: Color,
    tick: Color,
) {
    Canvas(Modifier.size(AVATAR_SIZE).background(background)) {
        val w = this.size.width
        val path =
            Path().apply {
                moveTo(w * 0.28f, w * 0.52f)
                lineTo(w * 0.44f, w * 0.67f)
                lineTo(w * 0.73f, w * 0.36f)
            }
        drawPath(path, tick, style = Stroke(width = w * 0.09f, cap = StrokeCap.Round, join = StrokeJoin.Round))
    }
}

@Composable
private fun PersonGlyph(
    background: Color,
    figure: Color,
) {
    Canvas(Modifier.size(AVATAR_SIZE).background(background)) {
        val w = this.size.width
        drawCircle(figure, radius = w * 0.17f, center = Offset(w / 2, w * 0.38f))
        // Shoulders: the top half of an oval, cut off by the circle's edge.
        drawArc(
            color = figure,
            startAngle = 180f,
            sweepAngle = 180f,
            useCenter = true,
            topLeft = Offset(w * 0.2f, w * 0.62f),
            size = Size(w * 0.6f, w * 0.5f),
        )
    }
}

/** One avatar colour, for light and dark mode, each with an initials colour that meets WCAG AA. */
data class AvatarColors(
    val lightBackground: Color,
    val lightText: Color,
    val darkBackground: Color,
    val darkText: Color,
)

private val ON_DARK_TINT = Color(0xFF1B1B1F)

/** Checked for AA contrast (4.5:1) in `SenderAvatarTest`. */
val AVATAR_PALETTE: List<AvatarColors> =
    listOf(
        0xFF1E5AA8 to 0xFFA8C7FA,
        0xFF7B3FA0 to 0xFFD7B8F0,
        0xFFA3324A to 0xFFF5B3C1,
        0xFF0F6B5C to 0xFF8FD5C5,
        0xFF8A5100 to 0xFFF2C27B,
        0xFF3D6B1F to 0xFFB6D99A,
        0xFF5B4FC4 to 0xFFC3BEF7,
        0xFFA63E00 to 0xFFF7B98F,
    ).map { (light, dark) -> AvatarColors(Color(light), Color.White, Color(dark), ON_DARK_TINT) }

/**
 * Which [AVATAR_PALETTE] colour an address gets: FNV-1a over its lowercased UTF-8 bytes, so it is the
 * same on every start and device.
 */
fun avatarColorIndex(address: String): Int {
    var hash = 0x811C9DC5.toInt()
    for (byte in address.trim().lowercase(Locale.ROOT).toByteArray(Charsets.UTF_8)) {
        hash = (hash xor (byte.toInt() and 0xFF)) * 0x01000193
    }
    return Math.floorMod(hash, AVATAR_PALETTE.size)
}

/**
 * Up to two initials for a sender: from the display name, else the address's local part ("ana.lopez" gives
 * "AL"). Latin names give the first letters of the first and last words; other scripts give their first
 * grapheme ("山田太郎" gives "山"). Null when the name starts with no letter (emoji, symbols, empty), for the
 * generic person.
 */
fun avatarInitials(
    name: String,
    address: String,
): String? {
    val cleanName = name.trim().trim('"', '\'').trim()
    val source =
        when {
            cleanName.isNotEmpty() && '@' !in cleanName -> cleanName
            else -> address.substringBefore('@').ifBlank { cleanName.substringBefore('@') }
        }
    val words = source.split(Regex("[\\s._+,;()\\-]+")).filter { it.isNotEmpty() }
    val first = words.firstOrNull()?.let(::firstGrapheme) ?: return null
    if (!Character.isLetterOrDigit(first.codePointAt(0))) return null
    val latin = Character.UnicodeScript.of(first.codePointAt(0)) == Character.UnicodeScript.LATIN
    if (!latin || words.size < 2) return first.uppercase(Locale.ROOT)
    val last = firstGrapheme(words.last())
    val lastOk = last != null && Character.isLetter(last.codePointAt(0))
    return (first + if (lastOk) last else "").uppercase(Locale.ROOT)
}

/** The first user-perceived character of [text] (a letter with its accents, a whole emoji), or null. */
private fun firstGrapheme(text: String): String? {
    if (text.isEmpty()) return null
    val it = BreakIterator.getCharacterInstance()
    it.setText(text)
    val end = it.next()
    return if (end == BreakIterator.DONE || end <= 0) null else text.substring(0, end)
}
