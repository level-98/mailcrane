package app.mailcrane.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Eight distinct marker colours for accounts (M4-01), readable on light and dark backgrounds. */
private val ACCOUNT_COLORS =
    listOf(
        Color(0xFF1E88E5),
        Color(0xFFD81B60),
        Color(0xFF43A047),
        Color(0xFFFB8C00),
        Color(0xFF8E24AA),
        Color(0xFF00897B),
        Color(0xFFE53935),
        Color(0xFF6D4C41),
    )

/** The marker colour for colour index [index] (the core's `account_color`). */
fun accountMarkerColor(index: Int): Color = ACCOUNT_COLORS[Math.floorMod(index, ACCOUNT_COLORS.size)]

/** A small coloured dot marking which account something belongs to. Decorative; the text names the account. */
@Composable
fun AccountDot(
    colorIndex: Int,
    size: Dp = 10.dp,
) {
    Box(Modifier.size(size).background(accountMarkerColor(colorIndex), CircleShape))
}
