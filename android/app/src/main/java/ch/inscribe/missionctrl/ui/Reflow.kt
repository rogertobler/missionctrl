package ch.inscribe.missionctrl.ui

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.buildAnnotatedString

/** One line as the output view draws it. */
data class DisplayLine(
    val text: AnnotatedString,
    /** Columns to indent wrapped continuation lines by (under the text, not the margin). */
    val hang: Int = 0,
    /** Draw on one line and clip at the edge (separators). */
    val noWrap: Boolean = false,
)

/**
 * Fits terminal lines formatted for the laptop's pane width to a narrower phone.
 *
 * herdr's pane has one size, the laptop terminal's, and agents like Claude Code format their
 * output to it. Wrapping those lines again on the phone tears separators apart and sends
 * continuation lines to the margin. This keeps separators on one line, shrinks long runs of
 * spaces, joins paragraphs the agent wrapped at the pane width, and indents wrapped lines under
 * their text. Lines it is unsure about stay as they are.
 */
object Reflow {
    /** Glyphs IBM Plex Mono lacks, replaced by the closest one it has (same length). */
    private val glyphs = mapOf('⏵' to '»', '⏴' to '«', '⏺' to '●', '⏸' to '‖')

    private const val SEPARATORS = "─━═╌╍┄┅┈┉"
    private val listMarker = Regex("""^(\s*)([-*•●⏺⎿✻·]|\d+[.)]|[❯›>▸])\s+""")
    private val boxChars = Regex("[│┃┌┐└┘├┤┬┴┼╭╮╰╯║╔╗╚╝]")
    /** Code keeps its lines: assignments, statements, blocks, shell prompts, comments. */
    private val codeLike = Regex("""[{};=]|\$ |'''|^\s*(#|//)""")
    private val spaceRun = Regex(" {4,}")
    private val ruleRun = Regex("([$SEPARATORS])\\1{3,}")

    fun layout(lines: List<AnnotatedString>, phoneCols: Int): List<DisplayLine> {
        val fixed = lines.map(::fixGlyphs)
        val paneCols = fixed.maxOfOrNull { it.text.trimEnd().length } ?: 0
        val narrower = phoneCols in 1 until paneCols
        val out = ArrayList<DisplayLine>(fixed.size)
        var i = 0
        while (i < fixed.size) {
            var line = fixed[i]
            if (isSeparator(line.text)) { out += DisplayLine(shrinkRule(line, phoneCols), noWrap = true); i++; continue }
            // Join the agent's own wrapping back into one paragraph.
            if (narrower) {
                while (i + 1 < fixed.size && continues(line.text, fixed[i + 1].text, paneCols)) {
                    val next = fixed[i + 1]
                    val start = next.text.length - next.text.trimStart().length
                    val head = trimEnd(line)
                    // "Spiel-" + "Merker": the agent broke a hyphenated word, join without a space.
                    val hyphen = head.text.length >= 2 && head.text.last() == '-' && head.text[head.text.length - 2].isLetter()
                    // The joining space takes the style of the indent it replaces, so a coloured
                    // block (Claude's grey prompt box) has no holes.
                    val gap = if (start > 0) next.subSequence(start - 1, start) else AnnotatedString(" ")
                    line = buildAnnotatedString { append(head); if (!hyphen) append(gap); append(next.subSequence(start, next.length)) }
                    i++
                }
                if (line.text.trimEnd().length > phoneCols) line = shrinkSpaces(line, phoneCols)
            }
            out += DisplayLine(line, hang = hangOf(line.text).coerceAtMost((phoneCols / 2).coerceAtLeast(0)))
            i++
        }
        return out
    }

    private fun fixGlyphs(line: AnnotatedString): AnnotatedString {
        if (line.text.none { it in glyphs }) return line
        val text = String(CharArray(line.text.length) { glyphs[line.text[it]] ?: line.text[it] })
        return AnnotatedString(text, line.spanStyles, line.paragraphStyles)
    }

    fun isSeparator(text: String): Boolean {
        val t = text.trim()
        if (t.length < 10) return false
        return t.count { it in SEPARATORS }.toDouble() / t.length >= 0.7
    }

    /** Column where the text of a line starts, after indentation and a list marker. */
    fun hangOf(text: String): Int {
        val m = listMarker.find(text)
        return m?.value?.length ?: (text.length - text.trimStart().length)
    }

    /** [next] carries on the paragraph of [line], which the agent wrapped at [paneCols]. */
    fun continues(line: String, next: String, paneCols: Int): Boolean {
        val a = line.trimEnd()
        val b = next.trimEnd()
        if (a.isBlank() || b.isBlank()) return false
        if (a.length < paneCols - 14) return false // not full: the paragraph ended here
        if (isSeparator(a) || isSeparator(b)) return false
        if (boxChars.containsMatchIn(a) || boxChars.containsMatchIn(b)) return false
        if (listMarker.find(b) != null) return false // a new item, not a continuation
        if (codeLike.containsMatchIn(a) || codeLike.containsMatchIn(b)) return false
        // Aligned columns (tables, status lines) have runs of spaces inside.
        if (a.trimStart().contains("   ") || b.trimStart().contains("   ")) return false
        val indentB = b.length - b.trimStart().length
        return indentB == hangOf(a)
    }

    private fun trimEnd(s: AnnotatedString): AnnotatedString = s.subSequence(0, s.text.trimEnd().length)

    /**
     * Shortens the longest runs of line characters until a separator fits, so a label on it
     * (Claude puts the session name at the right end of its input box) stays visible.
     */
    fun shrinkRule(line: AnnotatedString, cols: Int): AnnotatedString {
        var s = line
        var over = s.text.trimEnd().length - cols
        while (cols > 0 && over > 0) {
            val run = ruleRun.findAll(s.text.trimEnd()).maxByOrNull { it.value.length } ?: break
            val cut = minOf(over, run.value.length - 3)
            if (cut <= 0) break
            val from = run.range.first + 3
            s = buildAnnotatedString { append(s.subSequence(0, from)); append(s.subSequence(from + cut, s.length)) }
            over -= cut
        }
        return s
    }

    /** Shortens the longest runs of spaces (at least 4) until the line fits, keeping two. */
    private fun shrinkSpaces(line: AnnotatedString, cols: Int): AnnotatedString {
        var s = line
        var over = s.text.trimEnd().length - cols
        while (over > 0) {
            val run = spaceRun.findAll(s.text.trimEnd()).maxByOrNull { it.value.length } ?: break
            val cut = minOf(over, run.value.length - 2)
            if (cut <= 0) break
            val from = run.range.first + 2
            s = buildAnnotatedString { append(s.subSequence(0, from)); append(s.subSequence(from + cut, s.length)) }
            over -= cut
        }
        return s
    }
}
