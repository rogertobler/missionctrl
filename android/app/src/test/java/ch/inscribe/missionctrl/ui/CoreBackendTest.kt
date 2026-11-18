package ch.inscribe.missionctrl.ui

import android.view.KeyEvent
import ch.inscribe.missionctrl.SavedServer
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.Connection
import ch.inscribe.missionctrl.core.DetectedBackend
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreBackendTest {
    private fun server(id: String, host: String = "a.example.com", backend: String = SavedServer.BACKEND_AUTO) =
        SavedServer(id, id, host, 22, "dev", null, "k", backend = backend)

    private val tui = DetectedBackend("tui", true)
    private val core = DetectedBackend("core", true)
    private val coreStopped = DetectedBackend("core", false)

    @Test fun oneRunningHerdrIsSavedAsAsked() {
        assertEquals(DetectOutcome.Save, Backends.outcome(listOf(tui)))
        assertEquals(DetectOutcome.Save, Backends.outcome(listOf(core)))
    }

    @Test fun bothHerdrAskWhich() {
        assertEquals(DetectOutcome.AskWhich, Backends.outcome(listOf(tui, core)))
        // Installed only still counts: the sheet says it starts when chosen.
        assertEquals(DetectOutcome.AskWhich, Backends.outcome(listOf(tui, coreStopped)))
    }

    @Test fun anInstalledCoreAloneIsNeverSavedWithoutAsking() {
        assertEquals(DetectOutcome.AskStartCore, Backends.outcome(listOf(coreStopped)))
    }

    @Test fun anUnclearProbeNeverSavesOrAsks() {
        val slow = DetectedBackend("tui", false, unclear = true)
        assertEquals(DetectOutcome.Stay, Backends.outcome(listOf(slow)))
        assertEquals(DetectOutcome.Stay, Backends.outcome(listOf(slow, coreStopped)))
        assertEquals(DetectOutcome.Stay, Backends.outcome(listOf(core, DetectedBackend("tui", false, unclear = true))))
        assertTrue(Backends.status(listOf(slow, core)) is Detection.Unclear)
    }

    @Test fun statusSaysWhatWasFound() {
        assertEquals(Detection.None, Backends.status(emptyList()))
        assertEquals(Detection.Found(listOf(tui)), Backends.status(listOf(tui)))
    }

    @Test fun onlyAutoWithANewAddressDetects() {
        val saved = server("1", host = "a.example.com")
        assertTrue(Backends.mustDetect(null, SavedServer.BACKEND_AUTO, "a.example.com", "22", "dev", ""))
        assertFalse(Backends.mustDetect(null, SavedServer.BACKEND_TUI, "a.example.com", "22", "dev", ""))
        assertFalse(Backends.mustDetect(saved, SavedServer.BACKEND_AUTO, " a.example.com ", "22", "dev", ""))
        assertTrue(Backends.mustDetect(saved, SavedServer.BACKEND_AUTO, "b.example.com", "22", "dev", ""))
        assertTrue(Backends.mustDetect(saved, SavedServer.BACKEND_AUTO, "a.example.com", "2222", "dev", ""))
        assertTrue(Backends.mustDetect(saved, SavedServer.BACKEND_AUTO, "a.example.com", "22", "ops", ""))
        assertTrue(Backends.mustDetect(saved, SavedServer.BACKEND_AUTO, "a.example.com", "22", "dev", "work"))
        // A fixed type switched back to Auto looks again.
        assertTrue(Backends.mustDetect(server("2", backend = SavedServer.BACKEND_CORE), SavedServer.BACKEND_AUTO, "a.example.com", "22", "dev", ""))
    }

    @Test fun absenceIsNamedPerType() {
        val absent = Connection.Reconnecting(1u, "socket refused", 1000uL, true)
        assertEquals(R.string.conn_tui_not_running, connectionShortRes(absent, SavedServer.BACKEND_TUI))
        assertEquals(R.string.conn_core_not_found, connectionShortRes(absent, SavedServer.BACKEND_CORE))
        assertEquals(R.string.conn_no_herdr, connectionShortRes(absent, SavedServer.BACKEND_AUTO))
        assertEquals(R.string.conn_core_not_found_text, connectionLongRes(absent, SavedServer.BACKEND_CORE))
        // A session that failed is not absence: the real error stays.
        val failed = Connection.Reconnecting(1u, "connection reset", 1000uL, false)
        assertEquals(R.string.workspaces_conn_reconnecting, connectionShortRes(failed, SavedServer.BACKEND_TUI))
        assertNull(connectionLongRes(failed, SavedServer.BACKEND_TUI))
    }

    @Test fun savedServersKeepTypeAndSocket() {
        val core = server("1", backend = SavedServer.BACKEND_CORE).copy(socketPath = "/tmp/hc.sock")
        assertEquals(core, SavedServer.fromJson(JSONObject(core.toJson().toString())))
        val auto = server("2")
        assertEquals(auto, SavedServer.fromJson(JSONObject(auto.toJson().toString())))
        // Entries from builds before the type: Auto, no socket.
        val old = JSONObject("""{"id":"3","name":"3","host":"a.example.com","port":22,"user":"dev","session":"","key_id":"k"}""")
        val read = SavedServer.fromJson(old)
        assertEquals(SavedServer.BACKEND_AUTO, read.backend)
        assertNull(read.socketPath)
    }

    @Test fun keyBarNamesBecomeTerminalBytes() {
        assertEquals("\r", TermKeys.named("enter"))
        assertEquals("\u001B[A", TermKeys.named("up"))
        assertEquals("\u001B[1;5D", TermKeys.named("ctrl+left"))
        assertEquals("\u001B[Z", TermKeys.named("shift+tab"))
        assertEquals("\u0003", TermKeys.named("ctrl+c"))
        assertEquals("\u001B\u0003", TermKeys.named("ctrlalt+c"))
        assertEquals(" ", TermKeys.named("space"))
        assertEquals("1", TermKeys.named("1"))
        assertNull(TermKeys.named("hyper+x"))
    }

    @Test fun aDetectionOnlyAppliesToTheFormItRanOn() {
        val ran = SavedServer(id = "s", name = "box", host = "box.example.com", port = 22, user = "me", session = null, keyId = "k", autoConnect = true, notify = true)
        assertTrue(Backends.stillMatches(ran, null, ran.copy(name = "renamed", notify = false), null))
        assertFalse(Backends.stillMatches(ran, null, ran.copy(host = "other.example.com"), null))
        assertFalse(Backends.stillMatches(ran, null, ran.copy(port = 2222), null))
        assertFalse(Backends.stillMatches(ran, null, ran.copy(keyId = null), null))
        assertFalse(Backends.stillMatches(ran, null, ran, "typed"))
        assertFalse(Backends.stillMatches(ran, null, null, null))
    }

    @Test fun shiftOnTheBarTypesCapitalsOrLeavesTheKeyToHerdr() {
        assertEquals("A", TermKeys.named("shift+a"))
        assertEquals("\u001B[1;2A", TermKeys.named("shift+up"))
        // No sure bytes: herdr encodes these for the program's keyboard mode.
        assertNull(TermKeys.named("shift+1"))
        assertNull(TermKeys.named("ctrl+shift+a"))
    }

    @Test fun applicationCursorModeChangesPlainArrowsOnly() {
        assertEquals("\u001BOA", TermKeys.named("up", appCursor = true))
        assertEquals("\u001BOH", TermKeys.named("home", appCursor = true))
        assertEquals("\u001B[1;5D", TermKeys.named("ctrl+left", appCursor = true))
        assertEquals("\r", TermKeys.named("enter", appCursor = true))
        // The soft and hardware keyboard follow the same mode.
        assertEquals("\u001BOA", TermKeys.bytes(KeyEvent.KEYCODE_DPAD_UP, 0, appCursor = true))
        assertEquals("\u001BOF", TermKeys.bytes(KeyEvent.KEYCODE_MOVE_END, 0, appCursor = true))
        assertEquals("\u001B[1;2A", TermKeys.bytes(KeyEvent.KEYCODE_DPAD_UP, 0, shift = true, appCursor = true))
        assertEquals("x", TermKeys.bytes(KeyEvent.KEYCODE_X, 'x'.code, appCursor = true))
    }

    @Test fun keysWithoutSureBytesAreLeftToHerdr() {
        // Modified Enter, Backspace, Tab and Esc have no legacy bytes of their own.
        for (k in listOf("ctrl+enter", "ctrl+backspace", "ctrl+tab", "alt+tab", "ctrl+esc",
            "ctrlalt+enter", "shift+esc", "ctrl+shift+enter")) {
            assertNull(k, TermKeys.named(k))
        }
        assertEquals("\t", TermKeys.named("tab"))
        assertEquals("\u007F", TermKeys.named("backspace"))
        assertEquals("\u001B[Z", TermKeys.named("shift+tab"))
    }

    @Test fun escAloneGoesAsItsByteAtOnce() {
        assertEquals("\u001B", TermKeys.named("esc"))
        assertEquals("\u001B", TermKeys.named("esc", appCursor = true))
    }

    @Test fun chordsWithSafeBytesGoAtOnce() {
        // Shift+Enter is LF: a new line in agent prompts, Enter in a shell, nothing left over.
        assertEquals("\n", TermKeys.named("shift+enter"))
        assertEquals("\u001B\r", TermKeys.named("alt+enter"))
        assertEquals("\u001B\u007F", TermKeys.named("alt+backspace"))
        assertEquals("\u007F", TermKeys.named("shift+backspace"))
        assertEquals("\u001B\u001B", TermKeys.named("alt+esc"))
        assertEquals("\n", TermKeys.safeChord(KeyEvent.KEYCODE_NUMPAD_ENTER, shift = true, alt = false, ctrl = false))
        assertEquals("\u001B\u007F", TermKeys.safeChord(KeyEvent.KEYCODE_DEL, shift = false, alt = true, ctrl = false))
        assertNull(TermKeys.safeChord(KeyEvent.KEYCODE_ENTER, shift = true, alt = true, ctrl = false))
        assertNull(TermKeys.safeChord(KeyEvent.KEYCODE_ENTER, shift = false, alt = false, ctrl = true))
        assertNull(TermKeys.safeChord(KeyEvent.KEYCODE_TAB, shift = false, alt = true, ctrl = false))
        assertNull(TermKeys.safeChord(KeyEvent.KEYCODE_ESCAPE, shift = true, alt = false, ctrl = false))
        assertNull(TermKeys.safeChord(KeyEvent.KEYCODE_ENTER, shift = false, alt = false, ctrl = false))
    }

    @Test fun onlyAPlainEnterCountsAsSending() {
        assertTrue(TermKeys.hasEnter("\r".toByteArray()))
        assertTrue(TermKeys.hasEnter("ls\r".toByteArray()))
        assertTrue(TermKeys.hasEnter("\u001B\rx\r".toByteArray()))
        assertFalse(TermKeys.hasEnter("\u001B\r".toByteArray()))
        assertFalse(TermKeys.hasEnter("\n".toByteArray()))
        assertFalse(TermKeys.hasEnter("abc".toByteArray()))
    }

    @Test fun hardwareChordsWithoutBytesGoByName() {
        assertEquals("ctrl+alt+shift+enter", TermKeys.combo(KeyEvent.KEYCODE_NUMPAD_ENTER, shift = true, alt = true, ctrl = true))
        assertEquals("ctrl+backspace", TermKeys.combo(KeyEvent.KEYCODE_DEL, shift = false, alt = false, ctrl = true))
        assertEquals("shift+esc", TermKeys.combo(KeyEvent.KEYCODE_ESCAPE, shift = true, alt = false, ctrl = false))
        assertEquals("ctrl+tab", TermKeys.combo(KeyEvent.KEYCODE_TAB, shift = false, alt = false, ctrl = true))
        // Chords with safe bytes go as those (TermKeys.safeChord).
        assertNull(TermKeys.combo(KeyEvent.KEYCODE_ENTER, shift = true, alt = false, ctrl = false))
        assertNull(TermKeys.combo(KeyEvent.KEYCODE_ESCAPE, shift = false, alt = true, ctrl = false))
        // Shift+Tab, unmodified keys and keys with bytes of their own stay bytes.
        assertNull(TermKeys.combo(KeyEvent.KEYCODE_TAB, shift = true, alt = false, ctrl = false))
        assertNull(TermKeys.combo(KeyEvent.KEYCODE_ENTER, shift = false, alt = false, ctrl = false))
        assertNull(TermKeys.combo(KeyEvent.KEYCODE_DPAD_UP, shift = false, alt = false, ctrl = true))
        assertNull(TermKeys.combo(KeyEvent.KEYCODE_C, shift = false, alt = false, ctrl = true))
    }

    @Test fun nothingFoundSavesNothing() {
        assertEquals(DetectOutcome.Stay, Backends.outcome(emptyList()))
    }

    @Test fun tagsOnlyWhereAHostIsListedTwice() {
        val servers = listOf(
            server("1", backend = SavedServer.BACKEND_TUI),
            server("2", backend = SavedServer.BACKEND_CORE),
            server("3", host = "b.example.com", backend = SavedServer.BACKEND_CORE),
            server("4", host = "c.example.com"),
            server("5", host = "C.example.com"),
        )
        val tags = Backends.tagged(servers) { id -> if (id == "4") "tui" else null }
        assertEquals("tui", tags["1"])
        assertEquals("core", tags["2"])
        assertNull(tags["3"])
        // "auto": the herdr it is connected to; unknown while not connected.
        assertEquals("tui", tags["4"])
        assertNull(tags["5"])
    }

    @Test fun aGridThatFitsIsShownWhole() {
        assertEquals(1f, TermView.fitScale(400f, 300f, 500f, 600f))
        val o = TermView.offset(400f, 300f, 390f, 290f, 500f, 600f)
        assertEquals(0f, o.x); assertEquals(0f, o.y)
    }

    @Test fun aLargerGridFollowsTheCursor() {
        assertEquals(0.5f, TermView.fitScale(1000f, 400f, 500f, 600f))
        // Cursor near the right end: the view ends at the grid's edge.
        val right = TermView.offset(1000f, 2000f, 990f, 1950f, 500f, 600f)
        assertEquals(500f, right.x)
        assertEquals(1400f, right.y)
        // Cursor in the middle: centred across, in the lower part down.
        val mid = TermView.offset(1000f, 2000f, 500f, 1000f, 500f, 600f)
        assertEquals(250f, mid.x)
        assertEquals(1000f - 420f, mid.y)
    }
}
