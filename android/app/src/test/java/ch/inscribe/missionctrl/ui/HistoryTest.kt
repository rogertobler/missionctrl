package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HistoryTest {
    @Test fun olderIsWhatComesBeforeTheScreen() {
        assertEquals(listOf("a", "b"), PaneHistory.older("a\nb\nc\nd", "c\nd"))
        assertEquals(emptyList<String>(), PaneHistory.older("c\nd", "c\nd"))
    }

    @Test fun olderFindsAScreenThatMovedOn() {
        // The screen got a new line meanwhile: the history still ends with its start.
        val older = PaneHistory.older("a\nb\nc\nd\ne", "c\nd\ne\nf")
        assertEquals(listOf("a", "b"), older)
    }

    @Test fun transcriptMarkers() {
        val screen = listOf("⏺ text", "more", "────────────────", "  ctrl+o to toggle")
        assertTrue(TranscriptPages.inTranscript(screen))
        assertFalse(TranscriptPages.inTranscript(listOf("⏺ text", "> ")))
        assertEquals(2, TranscriptPages.contentEnd(screen))
        assertEquals(1, TranscriptPages.dumpStart(listOf("old", " ✻ Claude Code v2.1", "x")))
    }
}
