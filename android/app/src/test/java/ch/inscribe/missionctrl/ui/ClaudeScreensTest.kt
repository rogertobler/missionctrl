package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Claude Code (2.1.302) screens as recorded on 18.10.2026 in an isolated herdr with a play
 * project, manual mode: test resources screens/claude/.
 */
class ClaudeScreensTest {
    private fun screen(name: String) = javaClass.getResource("/screens/claude/$name.txt")!!.readText()
    private fun menu(name: String) = PaneTextTools.menu(PaneTextTools.question(screen(name)))!!

    @Test fun editApprovalWithoutTheDiff() {
        val m = menu("edit")
        assertEquals("Do you want to make this edit to calc.py?", m.question) // the "╌╌╌" line ends the diff
        assertEquals(listOf("Yes", "Yes, and switch to accept edits (auto-approve file edits and common file commands) for this session (shift+tab)", "No"), m.options.map { it.title })
        assertEquals(PaneTextTools.TextRoute.PLAIN to emptyList<String>(), PaneTextTools.route(screen("edit")))
    }

    @Test fun commandApprovalShowsTheCommand() {
        assertEquals("Run the calculator script\n$ python3 calc.py\nDo you want to proceed?", menu("command").question)
        assertEquals(4, menu("command").options.size)
        val wrapped = menu("command-wrapped")
        assertEquals("Run the script and check power and cube\n" +
            "$ python3 calc.py && python3 -c \"import calc; print(calc.power(2, 10), calc.cube(3))\"\nDo you want to proceed?", wrapped.question)
        // Option 2 is wrapped at the pane's width: one title again, nothing in its detail.
        assertTrue(wrapped.options[1].title, wrapped.options[1].title.endsWith("calc.cube(3))\\\"\" commands in /tmp/mct/play"))
        assertEquals("", wrapped.options[1].detail)
        assertEquals("Yes, and switch to auto mode · auto mode handles these prompts for you", wrapped.options[2].title)
    }

    @Test fun questionSeriesUpToSubmit() {
        val s = screen("question")
        val m = menu("question")
        assertEquals("Which operation should I add to calc.py next?", m.question)
        assertEquals("Power" to "power(a, b) returning a ** b", m.options[0].title to m.options[0].detail)
        assertEquals(4, PaneTextTools.freeText(m)?.number)
        assertEquals(5, PaneTextTools.chat(m)?.number)
        assertEquals(0 to 2, PaneTextTools.progress(s))
        // Typed text: cursor down to "Type something.", then the text (it replaces the row) and Enter.
        assertEquals(PaneTextTools.TextRoute.FREE_TEXT to listOf("down", "down", "down"), PaneTextTools.route(s))
        assertEquals(PaneTextTools.TextRoute.FREE_TEXT to emptyList<String>(), PaneTextTools.route(screen("type-cursor")))
        // Typed on the laptop already: the row shows the text, more text joins it.
        assertEquals("Cube (a ** 3)", menu("type-typed").options.single { it.selected }.title)

        val second = screen("question2")
        assertEquals("Should I add tests along with it?", menu("question2").question)
        assertEquals(1 to 2, PaneTextTools.progress(second))
        assertEquals(PaneTextTools.TextRoute.FREE_TEXT to listOf("down", "down"), PaneTextTools.route(second))

        val review = screen("review")
        assertEquals(2 to 2, PaneTextTools.progress(review))
        assertEquals(listOf("Which operation should I add to calc.py next?" to "Cube (a ** 3)", "Should I add tests along with it?" to "No"), PaneTextTools.review(review))
        assertEquals(listOf("Submit answers", "Cancel"), menu("review").options.map { it.title })
        assertTrue(PaneTextTools.activity(screen("answered"), working = false)!!.startsWith("You chose a cube operation (a ** 3) and no tests."))
        // Cursor on "Chat about this": Enter closes the question, the text goes to Claude as a message.
        assertEquals(PaneTextTools.TextRoute.CHAT to listOf("enter"), PaneTextTools.route(screen("chat-cursor")))
    }

    @Test fun planWithItsFileAndFeedback() {
        val s = screen("plan")
        val m = menu("plan")
        assertEquals("Claude has written up a plan and is ready to execute. Would you like to proceed?", m.question)
        assertEquals(listOf("Yes, and use auto mode", "Yes, manually approve edits", "Tell Claude what to change"), m.options.map { it.title })
        assertEquals("", m.options[2].detail) // not "shift+tab to approve …", "ctrl+g to edit in …"
        assertEquals("~/.claude/plans/plan-adding-cube-a-and-vivid-mango.md", PaneTextTools.planPath(s))
        assertEquals(PaneTextTools.TextRoute.FREE_TEXT to listOf("down", "down"), PaneTextTools.route(s))
        assertEquals(PaneTextTools.TextRoute.FREE_TEXT to emptyList<String>(), PaneTextTools.route(screen("tell-cursor")))
        assertEquals("Skip the README note.", menu("tell-typed").options.single { it.selected }.title)
    }

    @Test fun activityAndStatusLine() {
        assertEquals("Garnishing… 21s · ↓ 1.1k tokens · thinking with xhigh effort", PaneTextTools.activity(screen("working"), working = true))
        assertEquals("I added subtract(a, b) to calc.py:5, right after add. It returns a - b. Nothing else in the file was changed, and I haven't run it.",
            PaneTextTools.activity(screen("done"), working = false))
        assertEquals("I added power(a, b) and cube(a) to calc.py, right after multiply. cube works by calling power(a, 3).",
            PaneTextTools.activity(screen("plan-done"), working = false))
        // "⏸ manual mode on · ? for shortcuts · ← 1 agent" under the input is not part of a question.
        assertFalse(PaneTextTools.question(screen("done")).contains("manual mode on"))
        assertFalse(PaneTextTools.question(screen("working")).contains("/effort"))
    }

    /** "Copy" takes only the last answer: no turn line, input box or mode line. */
    @Test fun lastAnswerWithoutClaudesChrome() {
        assertEquals(
            "I added subtract(a, b) to calc.py:5, right after add. It returns a - b. Nothing else in the file was changed, and I\n" +
                "haven't run it.\n\n" +
                "calc.py already had uncommitted changes before I started. I left those alone, so if you commit now, your earlier\n" +
                "edits will be included.",
            PaneTextTools.lastAnswer(screen("done")),
        )
        // Claude Code 2.1.317, recorded 08.11.2026: auto mode, nbsp after the prompt.
        assertEquals("sleep 20 ist durchgelaufen. Danach hast du die Frage mit Grün\nbeantwortet.", PaneTextTools.lastAnswer(screen("copy-answer")))
    }

    /** The collapsed notification shows the question, not the last option. */
    @Test fun notificationHeadlineIsTheQuestion() {
        assertEquals("Should I add tests along with it?", PaneTextTools.headline(PaneTextTools.question(screen("question2"))))
        assertEquals("Do you want to proceed?", PaneTextTools.headline(PaneTextTools.question(screen("command"))))
        assertEquals("What next?", PaneTextTools.headline("Done.\nWhat next?"))
    }
}
