package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class NamesAndPathsTest {
    @Test fun titleAddsWorkspaceOnlyForDuplicates() {
        assertEquals("Stats", Names.title("Stats", "Score", "x", emptySet()))
        assertEquals("Stats · Score", Names.title("Stats", "Score", "x", setOf("stats")))
        assertEquals("Score", Names.title(null, "Score", "x", emptySet()))
    }

    @Test fun pathsAndSeparators() {
        assertEquals("~/src/app", PathLines.tilde("/home/me/src/app", "/home/me"))
        assertEquals("~", PathLines.tilde("/home/me", "/home/me/"))
        assertEquals("/home/meier", PathLines.tilde("/home/meier", "/home/me"))
        assertEquals("a/b", PathLines.join(listOf("a", "b"), "/"))
        assertEquals("a · b", PathLines.join(listOf("a", "b"), "·"))
    }

    @Test fun firstCharacterOfAnEmoji() {
        assertEquals("👩‍💻", Symbols.first("👩‍💻 rest"))
        assertEquals("🛠️", Symbols.first("🛠️"))
        assertEquals(null, Symbols.first("  "))
    }
}
