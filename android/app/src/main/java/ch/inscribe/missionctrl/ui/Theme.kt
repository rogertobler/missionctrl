package ch.inscribe.missionctrl.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import ch.inscribe.missionctrl.R

/** Default accent: mint. */
val DefaultAccent = Color(0xFF00E28A)

/** Workspace palette (same values as the design prototype). */
val WorkspacePalette = listOf(
    Color(0xFF5B8DEF), Color(0xFF4CB782), Color(0xFFE8833A), Color(0xFF9B7BEA),
    Color(0xFFE86A8A), Color(0xFFE3C34D), Color(0xFF4BC0C8), Color(0xFFA3A39F),
)

/** Accent choices offered in the settings. */
val AccentChoices = listOf(
    Color(0xFF00E28A), Color(0xFF4FB3FF), Color(0xFF8E7CFF), Color(0xFFFF6F91),
    Color(0xFFF2A93B), Color(0xFFE5E4DF),
)

@Immutable
data class McColors(
    val dark: Boolean,
    val bg: Color,
    val surface: Color,
    val inset: Color,
    val line: Color,
    val text: Color,
    val muted: Color,
    val mono: Color,
    val segActive: Color,
    val accent: Color,
    /** Accent usable as text/dots: darkened in light mode until it reaches 4.5:1. */
    val accentText: Color,
    /** Text/icons on an accent fill. */
    val onAccent: Color,
    /** Faint accent fill for "waiting" pills. */
    val accentSoft: Color,
    val danger: Color,
)

private fun mix(a: Color, b: Color, t: Float) = Color(
    red = a.red * (1 - t) + b.red * t,
    green = a.green * (1 - t) + b.green * t,
    blue = a.blue * (1 - t) + b.blue * t,
)

private fun contrastOnWhite(c: Color) = 1.05f / (c.luminance() + 0.05f)

fun mcColors(dark: Boolean, accent: Color): McColors {
    val bg = if (dark) Color(0xFF111113) else Color(0xFFF7F7F5)
    var accentText = accent
    if (!dark) {
        var t = 0f
        while (t < 0.8f && contrastOnWhite(mix(accent, Color.Black, t)) < 4.5f) t += 0.05f
        accentText = mix(accent, Color.Black, t)
    }
    return McColors(
        dark = dark,
        bg = bg,
        surface = if (dark) Color(0xFF1A1A1E) else Color.White,
        inset = if (dark) Color(0xFF24242A) else Color(0xFFF1F0EC),
        line = if (dark) Color(0xFF2A2A30) else Color(0xFFE4E3DF),
        text = if (dark) Color(0xFFECEAE6) else Color(0xFF18181A),
        muted = if (dark) Color(0xFF9A9893) else Color(0xFF66655F),
        mono = if (dark) Color(0xFFD9D6D0) else Color(0xFF2B2A27),
        segActive = if (dark) Color(0xFF2C2C33) else Color(0xFFECEBE7),
        accent = accent,
        accentText = accentText,
        onAccent = if (accent.luminance() > 0.35f) Color(0xFF04140D) else Color.White,
        accentSoft = if (dark) mix(accent, bg, 0.82f) else mix(accent, Color.White, 0.86f),
        danger = if (dark) Color(0xFFFF6B6B) else Color(0xFFC62828),
    )
}

val PlexSans = FontFamily(
    Font(R.font.ibm_plex_sans, FontWeight.Normal, variationSettings = FontVariation.Settings(FontVariation.weight(400))),
    Font(R.font.ibm_plex_sans, FontWeight.Medium, variationSettings = FontVariation.Settings(FontVariation.weight(500))),
    Font(R.font.ibm_plex_sans, FontWeight.SemiBold, variationSettings = FontVariation.Settings(FontVariation.weight(600))),
)

val PlexMono = FontFamily(
    Font(R.font.ibm_plex_mono_regular, FontWeight.Normal),
    Font(R.font.ibm_plex_mono_medium, FontWeight.Medium),
)

/** Small type scale used across the app. */
object McType {
    val title = TextStyle(fontFamily = PlexSans, fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
    val bigTitle = TextStyle(fontFamily = PlexSans, fontWeight = FontWeight.SemiBold, fontSize = 24.sp)
    val body = TextStyle(fontFamily = PlexSans, fontSize = 15.sp)
    val bodyStrong = TextStyle(fontFamily = PlexSans, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
    val small = TextStyle(fontFamily = PlexSans, fontSize = 13.sp)
    val caption = TextStyle(fontFamily = PlexSans, fontSize = 12.sp)
    val section = TextStyle(fontFamily = PlexSans, fontWeight = FontWeight.Medium, fontSize = 12.sp, letterSpacing = 0.96.sp)
    val path = TextStyle(fontFamily = PlexMono, fontSize = 11.5.sp)
    val mono = TextStyle(fontFamily = PlexMono, fontSize = 13.sp, lineHeight = 20.sp)
    val output = TextStyle(fontFamily = PlexMono, fontSize = 12.5.sp, lineHeight = 20.sp)
}

val LocalMc = staticCompositionLocalOf { mcColors(true, DefaultAccent) }

enum class ThemeMode { SYSTEM, LIGHT, DARK }

@Composable
fun McTheme(mode: ThemeMode, accent: Color, content: @Composable () -> Unit) {
    val dark = when (mode) {
        ThemeMode.SYSTEM -> isSystemInDarkTheme()
        ThemeMode.LIGHT -> false
        ThemeMode.DARK -> true
    }
    val c = mcColors(dark, accent)
    val scheme = (if (dark) darkColorScheme() else lightColorScheme()).copy(
        primary = c.accent,
        onPrimary = c.onAccent,
        background = c.bg,
        onBackground = c.text,
        surface = c.bg,
        onSurface = c.text,
        surfaceContainer = c.surface,
        surfaceContainerHigh = c.surface,
        surfaceContainerLow = c.surface,
        onSurfaceVariant = c.muted,
        outline = c.line,
        outlineVariant = c.line,
        error = c.danger,
    )
    CompositionLocalProvider(LocalMc provides c) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
