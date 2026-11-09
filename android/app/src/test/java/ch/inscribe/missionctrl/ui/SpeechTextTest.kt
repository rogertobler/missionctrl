package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.SpeechPrefs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SpeechTextTest {
    @Test fun replacementsOnWholeWordsIgnoringCase() {
        val p = SpeechPrefs(replacements = listOf("Herder" to "herdr", "Cloud" to "Claude"))
        assertEquals("herdr und Claude, nicht Clouds", SpeechText.process("herder und cloud, nicht Clouds", p))
    }

    @Test fun swissGermanWritesSs() {
        assertEquals("Strasse", SpeechText.process("Straße", SpeechPrefs(language = "de-CH")))
        assertEquals("Straße", SpeechText.process("Straße", SpeechPrefs(language = "de-DE")))
    }

    @Test fun readsVoxtypeConfig() {
        val toml = """
            [whisper]
            initial_prompt = "Claude Code, herdr, Proxmox."
            [text]
            replacements = { "herder" = "herdr", "docks" = "docs" }
        """.trimIndent()
        val (words, pairs) = SpeechText.fromVoxtype(toml)
        assertEquals(listOf("Claude Code", "herdr", "Proxmox"), words)
        assertEquals(listOf("herder" to "herdr", "docks" to "docs"), pairs)
    }

    @Test fun skipsCommentedVoxtypeExamples() {
        val toml = """
            # initial_prompt = "Beispiel"
            [whisper]
            initial_prompt = "herdr."
            # [text]
            # replacements = { "hyperwhisper" = "hyprwhspr" }
            [text]
            replacements = { "Herder" = "herdr" }
        """.trimIndent()
        assertEquals(listOf("herdr") to listOf("Herder" to "herdr"), SpeechText.fromVoxtype(toml))
    }

    @Test fun promptKeepsWordsAndTheEndOfWhatWasHeard() {
        assertEquals("herdr, Proxmox", SpeechText.prompt(listOf("herdr", "Proxmox"), emptyList()))
        assertEquals("herdr Erster Satz. Zweiter Satz.", SpeechText.prompt(listOf("herdr"), listOf("Erster Satz.", "Zweiter Satz.")))
        assertEquals("Nur Text.", SpeechText.prompt(emptyList(), listOf("Nur Text.")))
        val long = SpeechText.prompt(emptyList(), listOf("a".repeat(150), "b".repeat(150)))
        assertEquals(200, long.length)
        assertTrue(long.endsWith("b".repeat(150)))
    }

    @Test fun partsEndAtPauses() {
        val second = WhisperRecorder.RATE * 2
        assertTrue(WhisperRecorder.endsPart(3 * second, 700))
        assertFalse(WhisperRecorder.endsPart(3 * second, 300)) // a comma, not a sentence
        assertTrue(WhisperRecorder.endsPart(13 * second, 250)) // long part: a shorter pause will do
        assertFalse(WhisperRecorder.endsPart(13 * second, 100))
        assertTrue(WhisperRecorder.endsPart(25 * second, 0)) // at the latest
    }

    /** 100 ms frames of silence or speech; the splitter only sees the flag. */
    private fun PartSplitter.feed(frames: Int, speech: Boolean): List<ByteArray> {
        val frame = ByteArray(WhisperRecorder.RATE / 5)
        return (1..frames).mapNotNull { add(frame, frame.size, speech) }
    }

    private fun seconds(part: ByteArray) = part.size / (WhisperRecorder.RATE * 2.0)

    @Test fun longSilenceStaysOutOfTheNextPart() {
        // 30 s silence, 1 s speech, 0.7 s pause gave a 30.1 s part and cut the word.
        val s = PartSplitter()
        assertTrue(s.feed(300, speech = false).isEmpty())
        assertTrue(s.feed(10, speech = true).isEmpty())
        val parts = s.feed(7, speech = false)
        assertEquals(1, parts.size)
        assertEquals(2.2, seconds(parts[0]), 0.05) // 0.5 s lead-in + 1 s speech + 0.7 s pause
        assertEquals(0, s.rest().size)
    }

    @Test fun sentencesBecomePartsAndSpeechAtTheEndIsTheRest() {
        val s = PartSplitter()
        val first = s.feed(20, speech = true) + s.feed(7, speech = false)
        assertEquals(1, first.size)
        assertTrue(s.feed(150, speech = false).isEmpty()) // a long pause between sentences
        assertTrue(s.feed(15, speech = true).isEmpty())
        assertEquals(2.0, seconds(s.rest()), 0.05) // 0.5 s lead-in + 1.5 s speech, no pause yet
    }

    @Test fun stopAfterPauseIsAChoice() {
        assertEquals(3_500, SpeechPrefs().stopAfterMs)
        assertTrue(SpeechPrefs.STOP_AFTER_MS in SpeechPrefs.STOP_CHOICES)
        assertTrue(0 in SpeechPrefs.STOP_CHOICES) // only with ✓
    }

    @Test fun claudeServiceIsUnusable() {
        assertTrue(SpeechPrefs.unusable("service:com.anthropic.claude/com.anthropic.claude.bell.assist.ClaudeRecognitionService"))
        assertFalse(SpeechPrefs.unusable("service:com.google.android.tts/com.google.android.apps.speech.tts.googletts.service.GoogleTTSRecognitionService"))
        assertFalse(SpeechPrefs.unusable("device"))
        assertFalse(SpeechPrefs.unusable("whisper"))
    }
}
