package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PaneTextToolsTest {
    private val planApproval = """
        ⏺ Here is the plan.

        ╭──────────────────────────────────────╮
        │ Would you like to proceed?            │
        │                                       │
        │ ❯ 1. Yes, and auto-accept edits       │
        │   2. Yes, and manually approve edits  │
        │   3. No, keep planning                │
        ╰──────────────────────────────────────╯
    """.trimIndent()

    @Test fun recognisesChoiceMenu() {
        assertTrue(PaneTextTools.showsMenu(planApproval))
        assertEquals(QuestionKind.MENU, PaneTextTools.kind(PaneTextTools.question(planApproval)))
    }

    @Test fun numberedListWithoutSelectionIsNoMenu() {
        val text = "Steps:\n1. build\n2. test\n3. ship\n> "
        assertFalse(PaneTextTools.showsMenu(text))
    }

    @Test fun optionsInScreenOrder() {
        val options = PaneTextTools.options(PaneTextTools.question(planApproval))
        assertEquals(listOf("1", "2", "3"), options.map { it.first })
        assertEquals("Yes, and auto-accept edits", options[0].second)
    }

    @Test fun questionDropsEmptyPromptAndBorders() {
        val q = PaneTextTools.question("What next?\n╭────╮\n│ >  │\n╰────╯\n? for shortcuts")
        assertEquals("What next?", q)
        assertEquals(QuestionKind.TEXT, PaneTextTools.kind(q))
    }

    @Test fun lastAnswerStartsAtLastMarker() {
        val text = "⏺ first\nmore\n> do it\n⏺ second answer\n  line two"
        assertEquals("second answer\nline two", PaneTextTools.lastAnswer(text))
    }

    /** "Copy" once took "✻ Brewed for 5s", the empty prompt and the mode line along. */
    @Test fun lastAnswerIsOnlyTheAnswer() {
        val sep = "─".repeat(65)
        val text = "● User answered Claude's questions:\n  ⎿  · Was ist deine Lieblingsfarbe? → Rot\n\n" +
            "● Du hast Rot gewählt.\n\n✻ Brewed for 5s · done 2:33 PM · 1 shell still running\n\n" +
            "$sep\n❯\u00a0\n$sep\n  ⏵⏵ auto mode on (shift+tab to cycle)\n"
        assertEquals("Du hast Rot gewählt.", PaneTextTools.lastAnswer(text))
    }

    @Test fun lastAnswerStopsAtAFramedInputBox() {
        assertEquals("Done.", PaneTextTools.lastAnswer("⏺ Done.\n\n╭────────────╮\n│ >          │\n╰────────────╯\n  ? for shortcuts"))
    }

    @Test fun lastAnswerKeepsTablesAndIndentedLists() {
        val text = "● Zwei Punkte:\n  • eins\n    weiter\n  • zwei\n\n  ┌─────┬─────────┐\n  │ a   │ b       │\n  ├─────┼─────────┤\n  │ 1   │ 2       │\n  └─────┴─────────┘\n\n✻ Worked for 4s"
        assertEquals(
            "Zwei Punkte:\n• eins\n  weiter\n• zwei\n\n┌─────┬─────────┐\n│ a   │ b       │\n├─────┼─────────┤\n│ 1   │ 2       │\n└─────┴─────────┘",
            PaneTextTools.lastAnswer(text),
        )
    }
}

class MenuQuestionTest {
    @Test fun longMenuIsShownWhole() {
        val screen = """
⏺ Erklärung davor
  noch mehr Text

Wie gehen wir mit dem FA-Cup um?

❯ 1. Jetzt umbauen
     Liga-Spiele und FA-Cup trennen.
  2. Nur markieren
     FA-Cup-Spiel in die Bilanz.
  3. Später entscheiden
     Jetzt nichts ändern.
  4. Type something.
  5. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel
""".trimIndent()
        val q = PaneTextTools.question(screen)
        assertEquals("Wie gehen wir mit dem FA-Cup um?", q.lines().first())
        assertEquals("  5. Chat about this", q.lines().last())
        assertEquals(QuestionKind.MENU, PaneTextTools.kind(q))
        assertEquals(5, PaneTextTools.options(q).size)
    }

    @Test fun menuSplitsQuestionAndOptions() {
        val q = "Der FA Cup ist getrackt, diese Spiele zählten dann in die\nöffentliche Bilanz. Wie soll es sein?\n" +
            "  1. Nicht tracken (Empfohlen)\n     Ein Spiel zählt nur, wenn beide\n     Teams aus Ligen kommen.\n" +
            "❯ 2. Normal tracken\n     Zählen wie jedes andere.\n  3. Type something."
        val m = PaneTextTools.menu(q)!!
        assertEquals("Der FA Cup ist getrackt, diese Spiele zählten dann in die öffentliche Bilanz. Wie soll es sein?", m.question)
        assertEquals(listOf(1, 2, 3), m.options.map { it.number })
        assertEquals("Ein Spiel zählt nur, wenn beide Teams aus Ligen kommen.", m.options[0].detail)
        assertEquals(listOf(false, true, false), m.options.map { it.selected })
        assertEquals(listOf("up", "enter"), PaneTextTools.choose(m, m.options[0]))
        assertEquals(listOf("enter"), PaneTextTools.choose(m, m.options[1]))
        assertEquals(listOf("down", "enter"), PaneTextTools.choose(m, m.options[2]))
        assertEquals(null, PaneTextTools.menu("nur Text\n1. eins"))
    }

    @Test fun chooseNowCountsFromTheCurrentCursorAndRefusesAnotherQuestion() {
        val seen = PaneTextTools.menu("Weiter?\n❯ 1. Ja\n  2. Nein\n  3. Später")!!
        val picked = seen.options[2] // "3. Später", seen with the cursor on 1: down, down, enter
        assertEquals(listOf("down", "down", "enter"), PaneTextTools.choose(seen, picked))
        // Meanwhile the cursor moved to 2 on the laptop: one step is enough.
        assertEquals(listOf("down", "enter"), PaneTextTools.chooseNow("Weiter?\n  1. Ja\n❯ 2. Nein\n  3. Später", picked))
        // Another question with other options: nothing is sent.
        assertEquals(null, PaneTextTools.chooseNow("Löschen?\n❯ 1. Ja\n  2. Nein", picked))
        assertEquals(null, PaneTextTools.chooseNow("Fertig.", picked))
    }

    @Test fun typedTextGoesToClaudesFreeTextRow() {
        val screen = "In welcher Sprache soll der Beitrag gehen?\n❯ 1. Englisch (Empfohlen)\n     Eine Nachricht: Titel, zwei Sätze.\n" +
            "  2. Deutsch\n  3. Mehrere Sprachen\n  4. Type something.\n  5. Chat about this"
        val menu = PaneTextTools.menu(PaneTextTools.question(screen))
        assertEquals(4, PaneTextTools.freeText(menu)?.number)
        assertEquals(5, PaneTextTools.chat(menu)?.number)
        val free = PaneTextTools.TextRoute.FREE_TEXT
        assertEquals(free to listOf("down", "down", "down"), PaneTextTools.route(screen))
        // Cursor already on the row: nothing to move.
        val onFour = screen.replace("❯ 1.", "  1.").replace("  4. Type", "❯ 4. Type")
        assertEquals(free to emptyList<String>(), PaneTextTools.route(onFour))
        // On "Chat about this": pick it, then the text is a chat message.
        val onFive = screen.replace("❯ 1.", "  1.").replace("  5. Chat", "❯ 5. Chat")
        assertEquals(PaneTextTools.TextRoute.CHAT to listOf("enter"), PaneTextTools.route(onFive))
        // A plain menu or no menu: plain input.
        assertEquals(PaneTextTools.TextRoute.PLAIN to emptyList<String>(), PaneTextTools.route("Weiter?\n❯ 1. Ja\n  2. Nein"))
        assertEquals(PaneTextTools.TextRoute.PLAIN to emptyList<String>(), PaneTextTools.route("Fertig."))
        // ↓ from 3 lands on 4, ↓ from 4 on 5, ↑ from 4 on 3.
        val onThree = PaneTextTools.menu(PaneTextTools.question(screen.replace("❯ 1.", "  1.").replace("  3. Mehrere", "❯ 3. Mehrere")))
        assertEquals(true, PaneTextTools.landsOnText(onThree, "down"))
        assertEquals(true, PaneTextTools.landsOnText(PaneTextTools.menu(PaneTextTools.question(onFour)), "down"))
        assertEquals(false, PaneTextTools.landsOnText(PaneTextTools.menu(PaneTextTools.question(onFour)), "up"))
    }

    @Test fun aSeriesOfQuestionsGoesOnUntilSubmit() {
        val q1 = "← ☐ Telegram  ☐ Tags  ☐ Versand  ☐ URL  ✔ Submit →\n\nSprache?\n❯ 1. Englisch\n  2. Deutsch\n  3. Type something.\n" +
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel"
        val q2 = "← ☒ Telegram  ☐ Tags  ☐ Versand  ☐ URL  ✔ Submit →\n\nKategorien für die Beiträge?\n❯ 1. 3 feste\n  2. Keine\n" +
            "Enter to select · Tab/Arrow keys to navigate · Esc to cancel"
        assertEquals(0 to 4, PaneTextTools.progress(q1))
        assertEquals(1 to 4, PaneTextTools.progress(q2))
        assertEquals(null, PaneTextTools.progress("Weiter?\n❯ 1. Ja\n  2. Nein"))
        // The next question is another one, the same one stays the same.
        val s1 = PaneTextTools.signature(PaneTextTools.question(q1))
        assertEquals(s1, PaneTextTools.signature(PaneTextTools.question(q1)))
        assertEquals(false, s1 == PaneTextTools.signature(PaneTextTools.question(q2)))
        assertEquals("Sprache?|1.Englisch|2.Deutsch|3.Type something.", s1)
    }

    @Test fun aFooterWrappedOnANarrowPaneIsNoOptionDetail() {
        // Claude Code 2.1.317 in a pane 56 columns wide (recorded 07.11.2026).
        val screen = "← ☐ Getränk  ☐ Ferien  ☐ Theme  ✔ Submit →\n\nKaffee oder Tee?\n\n❯ 1. Kaffee\n     Schwarz, stark, morgens\n" +
            "  2. Tee\n     Mild, warm, jederzeit\n  3. Type something.\n" + "─".repeat(56) + "\n  4. Chat about this\n\n" +
            "Enter to select · Tab/Arrow keys to navigate · Esc to\ncancel\n"
        val menu = PaneTextTools.menu(PaneTextTools.question(screen))!!
        assertEquals("Chat about this", menu.options.last().title)
        assertEquals("", menu.options.last().detail)
        assertEquals("Schwarz, stark, morgens", menu.options.first().detail)
        // Without a footer, the last option keeps its own lines.
        val plain = PaneTextTools.menu(PaneTextTools.question("Weiter?\n❯ 1. Ja\n  2. Nein\n     lieber nicht"))!!
        assertEquals("lieber nicht", plain.options.last().detail)
    }

    @Test fun submitShowsTheAnswersToReview() {
        val screen = "← ☒ Telegram  ☒ Tags  ☒ Versand  ☒ URL  ✔ Submit →\n\nReview your answers\n\n" +
            " │ ● In welcher Sprache soll der Beitrag in die Telegram-Gruppe\n │   t.me/ExampleLabs gehen? Es ist eine Gruppe für alle.\n" +
            "   → wir posten nicht automatisch\n ● Kategorien für die Beiträge?\n   → 3 feste, nur Etikett (Empfohlen)\n" +
            " ● Welche Adresse?\n   → /updates (Empfohlen)\n\nReady to submit your answers?\n\n❯ 1. Submit answers\n  2. Cancel"
        assertEquals(
            listOf(
                "In welcher Sprache soll der Beitrag in die Telegram-Gruppe t.me/ExampleLabs gehen? Es ist eine Gruppe für alle." to "wir posten nicht automatisch",
                "Kategorien für die Beiträge?" to "3 feste, nur Etikett (Empfohlen)",
                "Welche Adresse?" to "/updates (Empfohlen)",
            ),
            PaneTextTools.review(screen),
        )
        assertEquals(4 to 4, PaneTextTools.progress(screen))
        assertEquals("Ready to submit your answers?", PaneTextTools.menu(PaneTextTools.question(screen))?.question)
        assertEquals(null, PaneTextTools.review("Weiter?\n❯ 1. Ja\n  2. Nein"))
    }

    @Test fun planQuestionsShowTheQuestionAndFindThePlan() {
        val screen = "  - Weekly report stays\n Note from the review (taken over): weekly numbers are\n" +
            " ↓ Claude has written up a plan and is ready to execute. Would you like to proceed?\n\n" +
            " ❯ 1. Yes, and use auto mode\n   2. Yes, manually approve edits\n   3. Tell Claude what to change\n" +
            "      shift+tab to approve with this feedback ctrl+g to edit in Vim · ~/.claude/plans/happy-crafting-lecun.md"
        val q = PaneTextTools.question(screen)
        assertEquals("Claude has written up a plan and is ready to execute. Would you like to proceed?", PaneTextTools.menu(q)?.question)
        assertEquals("~/.claude/plans/happy-crafting-lecun.md", PaneTextTools.planPath(screen))
        assertEquals(3, PaneTextTools.freeText(PaneTextTools.menu(q))?.number)
        assertEquals(null, PaneTextTools.planPath("Weiter?\n❯ 1. Ja\n  2. Nein ~/x/plans/a.md"))
    }

    @Test fun notificationAnswersNeedTheSameOption() {
        val screen = "Weiter?\n❯ 1. Ja\n  2. Nein"
        assertEquals(true, PaneTextTools.stillOffers(screen, "2", "Nein"))
        assertEquals(false, PaneTextTools.stillOffers(screen, "2", "Später"))
        assertEquals(false, PaneTextTools.stillOffers("Fertig.", "1", "Ja"))
    }

    @Test fun unwrapJoinsTheAgentsWrapping() {
        val q = "Der FA Cup ist getrackt, diese Spiele zählten dann in die\nöffentliche Bilanz. Wie soll es sein?\n❯ 1. Nicht tracken (Empfohlen)\n  2. Normal tracken"
        assertEquals(
            "Der FA Cup ist getrackt, diese Spiele zählten dann in die öffentliche Bilanz. Wie soll es sein?\n❯ 1. Nicht tracken (Empfohlen)\n  2. Normal tracken",
            PaneTextTools.unwrap(q),
        )
    }
}

class ActivityTest {
    @Test fun workingShowsSpinnerLine() {
        val screen = "⏺ I'll check the types.\n\n✻ Type-checking… (13m 10s · ↓ 285.0k tokens · esc to interrupt)\n\n────────\n> \n────────"
        assertEquals("Type-checking… 13m 10s · ↓ 285.0k tokens", PaneTextTools.activity(screen, working = true))
    }

    @Test fun restingShowsStartOfLastAnswer() {
        val screen = "> do it\n⏺ Verstanden: Ich warte auf beide Prüfungen.\n\nZweiter Absatz\n────────\n> \n────────"
        assertEquals("Verstanden: Ich warte auf beide Prüfungen.", PaneTextTools.activity(screen, working = false))
    }

    @Test fun wrappedToolCallIsJoined() {
        val screen = "⏺ Bash(cd ~/projects/webshop &&\n      npm test)\n  ⎿  PASS\n────────\n> \n────────"
        assertEquals("Bash(cd ~/projects/webshop && npm test)", PaneTextTools.activity(screen, working = false))
    }

    @Test fun startBannerIsNoActivity() {
        val screen = " ▐▛███▜▌   Claude Code v2.1.299\n▝▜█████▛▘  Opus · ~/webshop\n────────\n> \n────────"
        assertEquals(null, PaneTextTools.activity(screen, working = false))
        // Marker scrolled off: the first line left on the screen.
        assertEquals("zurück.", PaneTextTools.activity("    zurück.\n  Das vierte Review\n────────", working = false))
        assertEquals("Session renamed", PaneTextTools.activity("  ⎿  Session renamed\n" + "─".repeat(30) + " Allgemein ─\n❯ ", working = false))
    }

    @Test fun nothingWithoutSpinner() {
        assertEquals(null, PaneTextTools.activity("plain shell\n$ ", working = true))
    }
}
