package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class LayoutErrorTextTest {
    /** Every code the core sends (`LayoutProblem::as_str`) has its own text. */
    @Test
    fun everyLayoutProblemHasAText() {
        val codes = listOf(
            "unreadable", "conflict", "partial", "child_elsewhere", "other_machine", "unclear",
            "gone", "busy", "unplaced", "backup", "too_large", "placed", "claimed",
        )
        for (code in codes) assertNotNull(code, layoutErrorText(code))
        assertNull(layoutErrorText("something_new"))
    }

    @Test
    fun newProblemsGetTheirOwnTexts() {
        assertEquals(R.string.core_layout_err_unplaced, layoutErrorText("unplaced"))
        assertEquals(R.string.core_layout_err_backup, layoutErrorText("backup"))
        assertEquals(R.string.core_layout_err_too_large, layoutErrorText("too_large"))
        assertEquals(R.string.core_layout_err_terminal_placed, layoutErrorText("placed"))
        assertEquals(R.string.core_layout_err_terminal_claimed, layoutErrorText("claimed"))
        assertEquals(R.string.core_layout_err_terminal_running, layoutErrorText("partial", closingTerminal = true))
        assertEquals(R.string.core_layout_err_partial, layoutErrorText("partial"))
    }
}
