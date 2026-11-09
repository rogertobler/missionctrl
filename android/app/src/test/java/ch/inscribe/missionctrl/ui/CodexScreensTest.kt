package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Codex screens as recorded on 18.10.2026 (codex-cli 0.163, herdr 0.9.5) in an isolated herdr
 * with a play project: test resources screens/codex/.
 */
class CodexScreensTest {
    private fun screen(name: String) = javaClass.getResource("/screens/codex/$name.txt")!!.readText()

    private fun moveCursor(screen: String, from: String, to: String) =
        screen.replace("› $from", "  $from").replace("  $to", "› $to")

    @Test fun approvalShowsItsQuestionAndPlainOptions() {
        val s = screen("approval")
        val menu = PaneTextTools.menu(PaneTextTools.question(s))!!
        assertTrue(menu.question, menu.question.startsWith("Would you like to make the following edits?"))
        assertEquals(listOf("Yes, proceed", "Yes, and don't ask again for these files", "No, and tell Codex what to do differently"), menu.options.map { it.title })
        assertEquals("", menu.options.last().detail) // the footer "Press enter to confirm …" is not part of it
        assertEquals(3, PaneTextTools.message(menu)?.number)
        assertTrue(PaneTextTools.takesText(menu, menu.options[2]))
        // Cursor on 1: typed text has no row to go to.
        assertEquals(PaneTextTools.TextRoute.PLAIN to emptyList<String>(), PaneTextTools.route(s))
        // Cursor on "No, and tell Codex …": pick it, then the text is a new message.
        val onThree = moveCursor(s, "1. Yes, proceed", "3. No, and tell")
        assertEquals(PaneTextTools.TextRoute.MESSAGE to listOf("enter"), PaneTextTools.route(onThree))
        // Notification buttons and the check before answering use the same titles.
        assertEquals(listOf("1" to "Yes, proceed", "2" to "Yes, and don't ask again for these files", "3" to "No, and tell Codex what to do differently"),
            PaneTextTools.options(PaneTextTools.question(s)))
    }

    @Test fun questionsSplitTitleAndExplanationAndCount() {
        val q1 = screen("question-1")
        val menu = PaneTextTools.menu(PaneTextTools.question(q1))!!
        assertEquals("How should users enter a calculation?", menu.question)
        assertEquals("Subcommands (Recommended)", menu.options[0].title)
        assertEquals("Use `calc.py add 2 3` or `calc.py divide 6 3`.", menu.options[0].detail)
        assertEquals(0 to 2, PaneTextTools.progress(q1))
        // "None of the above": cursor there, Tab opens the notes field, then the text.
        assertEquals(4, PaneTextTools.note(menu)?.number)
        assertEquals(PaneTextTools.TextRoute.NOTE to listOf("down", "down", "down", "tab"), PaneTextTools.route(q1))

        val q2 = screen("question-2")
        assertEquals(1 to 2, PaneTextTools.progress(q2))
        assertEquals("How should the CLI report invalid input or division by zero?", PaneTextTools.menu(PaneTextTools.question(q2))?.question)
        assertEquals(PaneTextTools.TextRoute.NOTE to listOf("down", "down", "tab"), PaneTextTools.route(q2))
        // Notes field already open on the row: just the text.
        val open = moveCursor(q2, "1. Plain text", "3. None of the above")
            .replace("tab to add notes | enter to submit all", "tab or esc to clear notes | enter to submit all")
        assertEquals(PaneTextTools.TextRoute.NOTE to emptyList<String>(), PaneTextTools.route(open))
        // ↓ from 3 "Expression" lands on "None of the above".
        val onThree = PaneTextTools.menu(PaneTextTools.question(moveCursor(q1, "1. Subcommands", "3. Expression")))
        assertTrue(PaneTextTools.landsOnText(onThree, "down"))
    }

    @Test fun trustAndPlanDialogs() {
        val trust = PaneTextTools.menu(PaneTextTools.question(screen("trust")))!!
        assertTrue(trust.question, trust.question.startsWith("Trust this folder?"))
        assertEquals(listOf("Trust and continue", "Back to Agent Command Center"), trust.options.map { it.title })

        val plan = screen("plan")
        val menu = PaneTextTools.menu(PaneTextTools.question(plan))!!
        assertEquals("Implement this plan?", menu.question)
        assertEquals("Yes, implement this plan", menu.options[0].title)
        assertEquals("Switch to Default and start coding", menu.options[0].detail)
        assertEquals(3, PaneTextTools.message(menu)?.number)
        assertTrue(PaneTextTools.asksToImplementPlan(plan))
        assertEquals(null, PaneTextTools.planPath(plan))
        // The plan in the pane (its heading "• Proposed Plan" is above the visible screen here).
        val inline = PaneTextTools.inlinePlan(plan.replace("  # Small CLI", "• Proposed Plan\n\n  # Small CLI"))!!
        assertTrue(inline, inline.startsWith("# Small CLI for calc.py"))
        assertTrue(inline, inline.contains("## Assumptions"))
        assertTrue(inline, !inline.contains("Implement this plan?"))
    }

    @Test fun activityInTheList() {
        assertEquals("Working 1s", PaneTextTools.activity(screen("working"), working = true))
        assertTrue(PaneTextTools.activity(screen("done"), working = false)!!.startsWith("Updated calc.py (calc.py) so divide() raises"))
        assertEquals("OpenAI Codex (v0.163.0)", PaneTextTools.activity(screen("idle"), working = false))
    }
}
