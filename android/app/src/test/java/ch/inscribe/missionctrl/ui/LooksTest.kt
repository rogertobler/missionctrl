package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.Look
import ch.inscribe.missionctrl.WorkspacePrefs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LooksTest {
    private val s = "srv"

    @Test fun tabColourBeforeWorkspaceBeforeNone() {
        var book = LookBook().withWorkspace(s, "w1", "Score", Look(color = 2))
        assertEquals(WorkspacePalette[2], book.paneColorOrNull(s, "w1", "Score", "Stats"))
        book = book.withTab(s, "w1", "Score", "Stats", Look(rgb = 0x112233))
        assertEquals(rgbColor(0x112233), book.paneColorOrNull(s, "w1", "Score", "Stats"))
        book = book.withWorkspace(s, "w1", "Score", Look(color = Look.NO_COLOR))
        assertNull(book.paneColorOrNull(s, "w1", "Score", "Other"))
        assertEquals(LookBook.NO_COLOR_GREY, book.workspaceColor(s, "w1", "Score"))
    }

    @Test fun automaticColourComesFromTheName() {
        assertEquals(WorkspacePalette[LookBook.autoColorIndex("Score")], LookBook().workspaceColor(s, "w1", "Score"))
    }

    @Test fun bindingToIdKeepsOtherWorkspacesOfTheSameName() {
        var book = LookBook().withPrefs(s, "w1", "Score", WorkspacePrefs(notify = false))
        book = book.withTab(s, "w2", "Score", "Stats", Look(color = 1))
        // w2 switches to its id: w1 (same name) keeps "no notifications", w2's tab look moves along.
        book = book.withPrefs(s, "w2", "Score", WorkspacePrefs(notify = false, bindToName = false))
        assertEquals(false, book.prefs(s, "w1", "Score").notify)
        assertEquals(true, book.prefs(s, "w1", "Score").bindToName)
        assertEquals(1, book.tab(s, "w2", "Score", "Stats").color)
    }

    @Test fun renamedTabKeepsItsLook() {
        val book = LookBook().withTab(s, "w1", "Score", "Stats", Look(icon = "emoji:🚀")).renamedTab(s, "w1", "Score", "Stats", "Statistik")
        assertEquals("emoji:🚀", book.tab(s, "w1", "Score", "Statistik").icon)
        assertNull(book.tab(s, "w1", "Score", "Stats").icon)
    }

    @Test fun emptyLookRemovesTheEntry() {
        val book = LookBook().withAgent("claude", Look(color = 1)).withAgent("claude", Look())
        assertEquals(emptyMap<String, Look>(), book.agents)
    }

    @Test fun agentNameBeforeType() {
        val book = LookBook().withAgent("claude", Look(color = 1)).withAgent("Reviewer", Look(color = 3))
        assertEquals(3, book.agent("claude", "Reviewer").color)
        assertEquals(1, book.agent("claude", null).color)
    }
}
