package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.SpeechPrefs
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class EntryTest {
    @Test fun fieldShownWhileTypingEvenIfEmpty() = assertTrue(Entry.TYPING.showsField(""))

    @Test fun heardFieldOnlyWithText() {
        assertTrue(Entry.HEARD.showsField("ja"))
        // Speech cancelled, sent on its own, or no microphone: back to the buttons.
        assertFalse(Entry.HEARD.showsField(" "))
    }

    @Test fun barNeverShowsField() = assertFalse(Entry.BAR.showsField("Entwurf"))

    @Test fun autoSendOnlyIntoEmptyField() {
        assertTrue(SpeechPrefs(autoSend = true).sendsHeard(""))
        assertFalse(SpeechPrefs(autoSend = true).sendsHeard("nimm B, weil"))
        assertFalse(SpeechPrefs(autoSend = false).sendsHeard(""))
    }
}
