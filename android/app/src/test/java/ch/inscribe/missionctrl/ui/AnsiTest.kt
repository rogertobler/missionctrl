package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AnsiTest {
    private val scheme = TerminalSchemes.MissionDark

    @Test fun stripRemovesColoursAndTitles() {
        assertEquals("red plain", Ansi.strip("\u001B[31mred\u001B[0m plain"))
        assertEquals("x", Ansi.strip("\u001B]0;title\u0007x"))
    }

    @Test fun linesCarryColours() {
        val line = Ansi.lines("\u001B[31mred\u001B[0m plain", scheme).single()
        assertEquals("red plain", line.text)
        assertTrue(line.spanStyles.any { it.start == 0 && it.end == 3 && it.item.color == scheme.color(1) })
    }

    @Test fun xterm256AndTruecolour() {
        val l256 = Ansi.lines("\u001B[38;5;196mx", scheme).single()
        assertTrue(l256.spanStyles.isNotEmpty())
        val rgb = Ansi.lines("\u001B[38;2;10;20;30my", scheme).single()
        assertEquals("y", rgb.text)
        assertTrue(rgb.spanStyles.isNotEmpty())
    }

    @Test fun oneAnnotatedLinePerTextLine() {
        assertEquals(3, Ansi.lines("a\nb\n\u001B[1mc", scheme).size)
    }
}
