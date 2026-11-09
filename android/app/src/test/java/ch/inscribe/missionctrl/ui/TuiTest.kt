package ch.inscribe.missionctrl.ui

import android.view.KeyEvent
import androidx.compose.ui.graphics.Color
import ch.inscribe.missionctrl.core.TermLine
import ch.inscribe.missionctrl.core.TermRun
import ch.inscribe.missionctrl.core.TermScreen
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TuiTest {
    private val scheme = TerminalSchemes.MissionDark

    private fun run(col: Int, text: String, fg: Int = -1, bg: Int = -1, width: Int = text.length, inverse: Boolean = false) =
        TermRun(col.toUShort(), width.toUShort(), text, fg, bg, false, false, false, false, inverse)

    @Test fun keysAsXtermSendsThem() {
        assertEquals("\r", TermKeys.bytes(KeyEvent.KEYCODE_ENTER, 0))
        assertEquals("\u007F", TermKeys.bytes(KeyEvent.KEYCODE_DEL, 0))
        assertEquals("\u001B[A", TermKeys.bytes(KeyEvent.KEYCODE_DPAD_UP, 0))
        assertEquals("\u001B[1;5C", TermKeys.bytes(KeyEvent.KEYCODE_DPAD_RIGHT, 0, ctrl = true))
        assertEquals("\u001B[Z", TermKeys.bytes(KeyEvent.KEYCODE_TAB, 0, shift = true))
        assertEquals("\u001BOP", TermKeys.bytes(KeyEvent.KEYCODE_F1, 0))
        assertEquals("\u001B[24~", TermKeys.bytes(KeyEvent.KEYCODE_F12, 0))
        assertEquals("\u001B[5~", TermKeys.bytes(KeyEvent.KEYCODE_PAGE_UP, 0))
        assertEquals("\u0003", TermKeys.bytes(KeyEvent.KEYCODE_C, 'c'.code, ctrl = true))
        assertEquals("\u001Bx", TermKeys.bytes(KeyEvent.KEYCODE_X, 'x'.code, alt = true))
        assertEquals("ä", TermKeys.bytes(0, 'ä'.code))
        assertNull(TermKeys.bytes(KeyEvent.KEYCODE_SHIFT_LEFT, 0))
        assertEquals("a\rb\r", TermKeys.text("a\nb\r\n"))
    }

    @Test fun messageOfSeveralLinesGoesAsOnePaste() {
        assertEquals("ja", TermKeys.message("ja"))
        assertEquals("\u001B[200~eins\nzwei\u001B[201~", TermKeys.message("eins\nzwei\n"))
        // A break only at the end makes no second line.
        assertEquals("ja", TermKeys.message("ja\n"))
    }

    @Test fun coloursFromTheCore() {
        assertNull(TermText.color(-1, scheme))
        assertEquals(scheme.color(1), TermText.color(1, scheme))
        assertEquals(Color(0x12, 0x34, 0x56), TermText.color(0x1000000 or 0x123456, scheme))
        val (fg, bg) = TermText.colors(run(0, "x", fg = 2, inverse = true), scheme)
        assertEquals(scheme.bg(), fg)
        assertEquals(scheme.color(2), bg)
    }

    @Test fun plainTextKeepsColumns() {
        val screen = TermScreen(
            10u, 2u,
            listOf(TermLine(listOf(run(0, "ab"), run(5, "日", width = 2), run(7, "c"))), TermLine(emptyList())),
            0u, 0u, true, 1u,
        )
        assertEquals("ab   日c", TermText.plain(screen))
    }
}
