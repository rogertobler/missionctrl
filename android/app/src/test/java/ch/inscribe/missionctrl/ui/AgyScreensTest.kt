package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * agy (Antigravity CLI 1.4.2) and Claude Code (2.1.302) screens as recorded on 18.10.2026 in an
 * isolated herdr with a play project: test resources screens/agy/ and screens/claude/.
 */
class AgyScreensTest {
    private fun screen(name: String) = javaClass.getResource("/screens/$name.txt")!!.readText()

    @Test fun trustQuestionsWithoutNumbersBecomeMenus() {
        val agy = PaneTextTools.menu(PaneTextTools.question(screen("agy/trust")))!!
        assertTrue(agy.question, agy.question.contains("Do you trust the contents of this project?"))
        assertEquals(listOf("Yes, I trust this folder", "No, exit"), agy.options.map { it.title })
        assertEquals(listOf(true, false), agy.options.map { it.selected })
        assertEquals(listOf("enter"), PaneTextTools.choose(agy, agy.options[0]))

        val claude = PaneTextTools.menu(PaneTextTools.question(screen("claude/trust")))!!
        assertTrue(claude.question, claude.question.contains("Is this a project you created or one you trust?"))
        assertEquals(listOf("No, exit", "Yes, I trust this folder"), claude.options.map { it.title })
        assertEquals(listOf("down", "enter"), PaneTextTools.choose(claude, claude.options[1]))
    }

    @Test fun commandApproval() {
        val s = screen("agy/command")
        val menu = PaneTextTools.menu(PaneTextTools.question(s))!!
        assertTrue(menu.question, menu.question.endsWith("Run this command?"))
        assertEquals("Yes, run command", menu.options[0].title)
        assertEquals("No, cancel", menu.options.last().title)
        assertEquals("", menu.options.last().detail) // not "↑/↓ Navigate · tab Amend …"
        assertEquals(PaneTextTools.TextRoute.PLAIN to emptyList<String>(), PaneTextTools.route(s))
    }

    @Test fun questionWithWriteIn() {
        val s = screen("agy/question")
        val menu = PaneTextTools.menu(PaneTextTools.question(s))!!
        assertEquals("Which mathematical operation would you like to add next to calc.py?", menu.question)
        assertEquals("(Recommended) Subtraction (`subtract(a, b)`)", menu.options[0].title)
        assertEquals(4, PaneTextTools.writeIn(menu)?.number)
        assertEquals(null, PaneTextTools.progress(s)) // "Question 1/1": one question, no series
        // Typed text: down to "Write-in...", Enter opens its field, then the text.
        assertEquals(PaneTextTools.TextRoute.MESSAGE to listOf("down", "down", "down", "enter"), PaneTextTools.route(s))
        assertTrue(PaneTextTools.landsOnText(PaneTextTools.menu(PaneTextTools.question(s.replace("> 1.", "  1.").replace("  3. Modulo", "> 3. Modulo"))), "down"))
        // Cursor on it: Enter still opens the field; open already: just the text.
        assertEquals(PaneTextTools.TextRoute.MESSAGE to listOf("enter"), PaneTextTools.route(screen("agy/writein-cursor")))
        assertEquals(PaneTextTools.TextRoute.MESSAGE to emptyList<String>(), PaneTextTools.route(screen("agy/writein-open")))
    }

    @Test fun activityInTheList() {
        assertEquals("Working...", PaneTextTools.activity(screen("agy/working"), working = true))
        assertEquals("Added the multiply function to calc.py:11-12.", PaneTextTools.activity(screen("agy/done"), working = false))
        assertEquals("Plan for Adding Square Root to calc.py", PaneTextTools.activity(screen("agy/answered"), working = false))
        assertEquals("Ran (python3 calc.py)", PaneTextTools.activity(screen("agy/ran"), working = false)) // without "(ctrl+o to expand)"
        val banner = "      ▄▀▀▄        Antigravity CLI 1.4.2\n     ▀▀▀▀▀▀       dev@example.com\n" + "─".repeat(40) + "\n>\n" + "─".repeat(40)
        assertEquals(null, PaneTextTools.activity(banner, working = false))
    }
}
