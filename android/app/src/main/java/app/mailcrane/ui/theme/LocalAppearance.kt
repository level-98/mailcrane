package app.mailcrane.ui.theme

import androidx.compose.runtime.staticCompositionLocalOf

/** Whether HTML mail should be darkened in dark mode ("Dark message bodies"). */
val LocalDarkMessageBodies = staticCompositionLocalOf { false }

/** Whether the app is showing its dark theme (which may differ from the system setting). */
val LocalDarkTheme = staticCompositionLocalOf { false }

/** How tightly the message list packs its rows (M4-24): Comfortable with sender pictures, or Compact. */
val LocalMailboxDensity = staticCompositionLocalOf { app.mailcrane.settings.MailboxDensity.COMFORTABLE }
