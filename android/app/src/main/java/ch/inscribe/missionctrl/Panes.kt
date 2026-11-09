package ch.inscribe.missionctrl

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import ch.inscribe.missionctrl.ui.ClaudeHistory
import ch.inscribe.missionctrl.ui.PaneHistory
import ch.inscribe.missionctrl.ui.TranscriptPages

/**
 * What the detail view keeps of one pane, owned by [Core] (per "server|pane") and dropped when
 * the pane is gone. Written from the reading coroutine, read by the views.
 */
class PaneView {
    /** Last read with colour codes, and its herdr revision (always 0 in herdr 0.9.5; the text decides). */
    var ansi by mutableStateOf<String?>(null)
    /** [ansi] without colour codes. */
    var plain by mutableStateOf("")
    var revision = -1L
    /** Plain lines older than the screen, for agents other than Claude ([PaneHistory]). */
    var olderPlain by mutableStateOf(emptyList<String>())
    var lastHarvest = 0L
    var harvestWanted = true
    /** Coloured history of fullscreen Claude. */
    val claude = ClaudeHistory()

    /** History and screen as plain text (copy button). */
    fun text(): String = PaneHistory.join(if (claude.lines.isNotEmpty()) TranscriptPages.plain(claude.older()) else olderPlain, plain)
}
