package ch.inscribe.missionctrl.ui

import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import ch.inscribe.missionctrl.R
import org.json.JSONArray
import org.json.JSONObject

/** A terminal colour scheme: background, foreground and the 16 ANSI colours. */
data class TerminalScheme(
    val id: String,
    val name: String,
    val background: Long,
    val foreground: Long,
    /** black, red, green, yellow, blue, magenta, cyan, white, then the bright variants. */
    val ansi: List<Long>,
    val custom: Boolean = false,
) {
    fun bg() = Color(background)
    fun fg() = Color(foreground)
    fun color(i: Int) = Color(ansi[i.coerceIn(0, 15)])

    /** The MissionCtrl schemes are named in the app's language; others (and own copies) keep [name]. */
    @get:StringRes val nameRes: Int?
        get() = if (custom) null else when (id) {
            "mc-dark" -> R.string.terminal_scheme_mc_dark
            "mc-light" -> R.string.terminal_scheme_mc_light
            else -> null
        }

    /** The name as the app shows it. */
    @Composable fun displayName(): String = nameRes?.let { stringResource(it) } ?: name

    fun toJson(): JSONObject = JSONObject()
        .put("id", id).put("name", name).put("bg", background).put("fg", foreground)
        .put("ansi", JSONArray(ansi))

    companion object {
        fun fromJson(j: JSONObject): TerminalScheme {
            val a = j.getJSONArray("ansi")
            return TerminalScheme(j.getString("id"), j.getString("name"), j.getLong("bg"), j.getLong("fg"), (0 until a.length()).map { a.getLong(it) }, custom = true)
        }
    }
}

private fun s(id: String, name: String, bg: Long, fg: Long, vararg ansi: Long) =
    TerminalScheme(id, name, 0xFF000000 or bg, 0xFF000000 or fg, ansi.map { 0xFF000000 or it })

/** Well-known schemes, values from their published palettes. */
object TerminalSchemes {
    val MissionDark = s("mc-dark", "MissionCtrl Dark", 0x1A1A1E, 0xD9D6D0,
        0x2A2A30, 0xE5636B, 0x3CCB8C, 0xE3C34D, 0x5B8DEF, 0xB48EF0, 0x4BC0C8, 0xD9D6D0,
        0x6B6A66, 0xFF7B83, 0x4BE3A3, 0xF2D36B, 0x7FA8FF, 0xC9A8FF, 0x6FD8DF, 0xFFFFFF)
    val MissionLight = s("mc-light", "MissionCtrl Light", 0xFFFFFF, 0x2B2A27,
        0x2B2A27, 0xC62828, 0x1B7F4C, 0x8A6D00, 0x2F5FC4, 0x7B4BC4, 0x13798A, 0x8A8984,
        0x66655F, 0xD84545, 0x24965C, 0xA88600, 0x3D72E0, 0x9461DC, 0x1A8FA3, 0x18181A)

    val presets: List<TerminalScheme> = listOf(
        MissionDark, MissionLight,
        s("solarized-dark", "Solarized Dark", 0x002B36, 0x839496,
            0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5,
            0x002B36, 0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3),
        s("solarized-light", "Solarized Light", 0xFDF6E3, 0x657B83,
            0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5,
            0x002B36, 0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3),
        s("dracula", "Dracula", 0x282A36, 0xF8F8F2,
            0x21222C, 0xFF5555, 0x50FA7B, 0xF1FA8C, 0xBD93F9, 0xFF79C6, 0x8BE9FD, 0xF8F8F2,
            0x6272A4, 0xFF6E6E, 0x69FF94, 0xFFFFA5, 0xD6ACFF, 0xFF92DF, 0xA4FFFF, 0xFFFFFF),
        s("nord", "Nord", 0x2E3440, 0xD8DEE9,
            0x3B4252, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x88C0D0, 0xE5E9F0,
            0x4C566A, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x8FBCBB, 0xECEFF4),
        s("gruvbox-dark", "Gruvbox Dark", 0x282828, 0xEBDBB2,
            0x282828, 0xCC241D, 0x98971A, 0xD79921, 0x458588, 0xB16286, 0x689D6A, 0xA89984,
            0x928374, 0xFB4934, 0xB8BB26, 0xFABD2F, 0x83A598, 0xD3869B, 0x8EC07C, 0xEBDBB2),
        s("one-dark", "One Dark", 0x282C34, 0xABB2BF,
            0x282C34, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF,
            0x5C6370, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xFFFFFF),
        s("tokyo-night", "Tokyo Night", 0x1A1B26, 0xC0CAF5,
            0x15161E, 0xF7768E, 0x9ECE6A, 0xE0AF68, 0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xA9B1D6,
            0x414868, 0xF7768E, 0x9ECE6A, 0xE0AF68, 0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xC0CAF5),
        s("catppuccin-mocha", "Catppuccin Mocha", 0x1E1E2E, 0xCDD6F4,
            0x45475A, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xBAC2DE,
            0x585B70, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xA6ADC8),
        s("catppuccin-latte", "Catppuccin Latte", 0xEFF1F5, 0x4C4F69,
            0x5C5F77, 0xD20F39, 0x40A02B, 0xDF8E1D, 0x1E66F5, 0xEA76CB, 0x179299, 0xACB0BE,
            0x6C6F85, 0xD20F39, 0x40A02B, 0xDF8E1D, 0x1E66F5, 0xEA76CB, 0x179299, 0xBCC0CC),
        s("monokai", "Monokai", 0x272822, 0xF8F8F2,
            0x272822, 0xF92672, 0xA6E22E, 0xF4BF75, 0x66D9EF, 0xAE81FF, 0xA1EFE4, 0xF8F8F2,
            0x75715E, 0xF92672, 0xA6E22E, 0xF4BF75, 0x66D9EF, 0xAE81FF, 0xA1EFE4, 0xF9F8F5),
    )

    /** Id meaning "MissionCtrl, matching light/dark mode". */
    const val AUTO = "auto"

    /** Names of the 16 ANSI colours (string resources), in [TerminalScheme.ansi] order. */
    val colorNameRes: List<Int> = listOf(
        R.string.terminal_color_black, R.string.terminal_color_red, R.string.terminal_color_green, R.string.terminal_color_yellow,
        R.string.terminal_color_blue, R.string.terminal_color_magenta, R.string.terminal_color_cyan, R.string.terminal_color_white,
        R.string.terminal_color_bright_black, R.string.terminal_color_bright_red, R.string.terminal_color_bright_green, R.string.terminal_color_bright_yellow,
        R.string.terminal_color_bright_blue, R.string.terminal_color_bright_magenta, R.string.terminal_color_bright_cyan, R.string.terminal_color_bright_white,
    )
}

/** Terminal settings: chosen scheme, own schemes, font size, and normal view or TUI. */
data class TerminalPrefs(
    val schemeId: String = TerminalSchemes.AUTO,
    val custom: List<TerminalScheme> = emptyList(),
    val fontSize: Float = 12.5f,
    /** Live terminal of the pane (Tui.kt) instead of the coloured history fitted to the phone. */
    val tui: Boolean = false,
    /** How long the TUI of a pane stays open after its page left; 0 closes at once. */
    val lingerSeconds: Int = 20,
) {
    fun scheme(dark: Boolean): TerminalScheme = when (schemeId) {
        TerminalSchemes.AUTO -> if (dark) TerminalSchemes.MissionDark else TerminalSchemes.MissionLight
        else -> custom.firstOrNull { it.id == schemeId } ?: TerminalSchemes.presets.firstOrNull { it.id == schemeId }
            ?: if (dark) TerminalSchemes.MissionDark else TerminalSchemes.MissionLight
    }

    companion object {
        const val MIN_FONT = 9f
        const val MAX_FONT = 22f
        /** "12.5 pt" / "12,5 pt": the decimal sign of [locale] (the app's language). */
        fun sizeLabel(size: Float, locale: java.util.Locale): String =
            java.text.NumberFormat.getNumberInstance(locale).apply { maximumFractionDigits = 1 }.format(size) + " pt"

        /** How big [size] is, as a string resource. */
        @StringRes fun sizeNameRes(size: Float): Int = when {
            size < 11f -> R.string.terminal_size_very_small
            size < 12.5f -> R.string.terminal_size_small
            size < 14f -> R.string.terminal_size_normal
            size < 16f -> R.string.terminal_size_large
            else -> R.string.terminal_size_very_large
        }
    }
}

/**
 * Turns text with ANSI SGR codes into styled lines. Handles the 16 colours,
 * 256 colours, true colour, bold/dim/italic/underline/inverse; other escape
 * sequences are dropped.
 */
object Ansi {
    private val esc = Regex("\u001B(\\[[0-9;?]*[ -/]*[@-~]|\\][^\u0007\u001B]*(\u0007|\u001B\\\\)|[@-Z\\\\-_])")

    fun strip(text: String): String = esc.replace(text, "")

    private data class State(
        var fg: Color? = null, var bg: Color? = null,
        var bold: Boolean = false, var dim: Boolean = false, var italic: Boolean = false,
        var underline: Boolean = false, var inverse: Boolean = false,
    )

    /** Levels of the xterm 6×6×6 colour cube. */
    private val CUBE = intArrayOf(0, 95, 135, 175, 215, 255)

    internal fun xterm256(n: Int, scheme: TerminalScheme): Color = when {
        n < 16 -> scheme.color(n)
        n < 232 -> {
            val i = n - 16
            Color(CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
        }
        else -> { val v = 8 + (n - 232) * 10; Color(v, v, v) }
    }

    private fun apply(params: List<Int>, st: State, scheme: TerminalScheme) {
        var i = 0
        val p = params.ifEmpty { listOf(0) }
        while (i < p.size) {
            when (val code = p[i]) {
                0 -> { st.fg = null; st.bg = null; st.bold = false; st.dim = false; st.italic = false; st.underline = false; st.inverse = false }
                1 -> st.bold = true
                2 -> st.dim = true
                3 -> st.italic = true
                4 -> st.underline = true
                7 -> st.inverse = true
                22 -> { st.bold = false; st.dim = false }
                23 -> st.italic = false
                24 -> st.underline = false
                27 -> st.inverse = false
                in 30..37 -> st.fg = scheme.color(code - 30)
                39 -> st.fg = null
                in 40..47 -> st.bg = scheme.color(code - 40)
                49 -> st.bg = null
                in 90..97 -> st.fg = scheme.color(code - 90 + 8)
                in 100..107 -> st.bg = scheme.color(code - 100 + 8)
                38, 48 -> {
                    val target: (Color) -> Unit = if (code == 38) { c -> st.fg = c } else { c -> st.bg = c }
                    if (p.getOrNull(i + 1) == 5 && i + 2 < p.size) { target(xterm256(p[i + 2], scheme)); i += 2 }
                    else if (p.getOrNull(i + 1) == 2 && i + 4 < p.size) { target(Color(p[i + 2].coerceIn(0, 255), p[i + 3].coerceIn(0, 255), p[i + 4].coerceIn(0, 255))); i += 4 }
                }
            }
            i++
        }
    }

    /** Parse into one AnnotatedString per line; style carries across lines. */
    fun lines(text: String, scheme: TerminalScheme): List<AnnotatedString> {
        val st = State()
        return text.split('\n').map { line -> line(line.trimEnd('\r'), st, scheme) }
    }

    private fun line(line: String, st: State, scheme: TerminalScheme): AnnotatedString = buildAnnotatedString {
        var pos = 0
        for (m in esc.findAll(line)) {
            if (m.range.first > pos) styled(line.substring(pos, m.range.first), st, scheme)
            val seq = m.value
            if (seq.startsWith("\u001B[") && seq.endsWith("m")) {
                val params = seq.substring(2, seq.length - 1).split(';').mapNotNull { it.toIntOrNull() }
                apply(params, st, scheme)
            }
            pos = m.range.last + 1
        }
        if (pos < line.length) styled(line.substring(pos), st, scheme)
    }

    private fun AnnotatedString.Builder.styled(text: String, st: State, scheme: TerminalScheme) {
        var fg = st.fg ?: scheme.fg()
        var bg = st.bg
        if (st.inverse) { val f = fg; fg = bg ?: scheme.bg(); bg = f }
        if (st.dim) fg = fg.copy(alpha = 0.6f)
        withStyle(
            SpanStyle(
                color = fg,
                background = bg ?: Color.Unspecified,
                fontWeight = if (st.bold) FontWeight.Medium else null,
                fontStyle = if (st.italic) FontStyle.Italic else null,
                textDecoration = if (st.underline) TextDecoration.Underline else null,
            ),
        ) { append(text) }
    }
}

/**
 * Joins the full history with the live coloured screen.
 *
 * Agents like Claude Code draw on the terminal's alternate screen, which has no scrollback.
 * herdr can still collect their history (it scrolls the agent while it rests), but only for
 * plain-text reads; a coloured (`ansi`) read returns just the screen. So the detail reads the
 * screen in colour all the time and the plain history now and then, and shows the part of the
 * history that lies above the screen.
 */
object PaneHistory {
    /** herdr caps a read at 1000 lines. */
    const val LINES = 1000
    /** Collecting the history scrolls the agent on the server, so not more often than this. */
    const val MIN_INTERVAL_MS = 8_000L

    /** Lines of [plain] (history ending with the screen) that are older than [screen]. */
    fun older(plain: String, screen: String): List<String> {
        val h = plain.trimEnd().lines().map { it.trimEnd() }
        val s = screen.trimEnd().lines().map { it.trimEnd() }
        if (s.isEmpty() || h.size <= s.size) return emptyList()
        // Usual case: the history ends with exactly the screen.
        if (h.takeLast(s.size) == s) return h.dropLast(s.size)
        // The screen changed in between: find its first lines near where it should start.
        val a = s.indexOfFirst { it.isNotBlank() }
        if (a < 0) return h.dropLast(s.size)
        val k = minOf(3, s.size - a)
        val probe = s.subList(a, a + k)
        val expected = h.size - s.size + a
        val match = (0..h.size - k).filter { h.subList(it, it + k) == probe }.minByOrNull { kotlin.math.abs(it - expected) }
        return if (match != null) h.subList(0, maxOf(0, match - a)) else h.dropLast(s.size)
    }

    /** Plain text of history plus screen, for question detection and copying. */
    fun join(older: List<String>, screen: String): String =
        if (older.isEmpty()) screen else older.joinToString("\n") + "\n" + screen
}
