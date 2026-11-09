package ch.inscribe.missionctrl.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.R
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout

/**
 * Coloured history of a Claude Code session in fullscreen rendering.
 *
 * Fullscreen Claude keeps its conversation on the alternate screen, so herdr has no coloured
 * scrollback for it. Claude's transcript view (`ctrl+o`) can be paged with `ctrl+u` (half a
 * screen up), and `[` there writes the whole conversation into the terminal's normal scrollback.
 * Both arrive in colour. The app takes the last 1000 lines with `[`, in one step; paging
 * further back with `ctrl+u` works too, but one step is simpler and enough. Only while Claude
 * rests, and always toggling back with `ctrl+o`, which returns Claude to its normal view,
 * following the bottom.
 *
 * herdr 0.9.5 does not accept `pageup`/`pagedown` through its API, hence the transcript keys.
 */
class ClaudeHistory {
    /** Raw ANSI lines, oldest first; the last [livePart] lines show what the screen showed. */
    val lines = mutableStateListOf<String>()
    var livePart by mutableIntStateOf(0)
    /** Content of the normal view when [lines] was taken; a change means new output. */
    var liveAtCapture: List<String> = emptyList()
    var busy by mutableStateOf(false)
    var note by mutableStateOf<HistoryNote?>(null)
    var lastAttempt = 0L

    fun clear() {
        lines.clear(); livePart = 0; liveAtCapture = emptyList(); note = null
    }

    /** Lines older than the live screen, for display above it. */
    fun older(): List<String> = lines.take((lines.size - livePart).coerceAtLeast(0))
}

/** A note at the top of the history: an error's message, or one of the app's own. */
sealed interface HistoryNote {
    data class Message(val text: String) : HistoryNote
    data object NotInNormalView : HistoryNote
    data object OnlyLastLines : HistoryNote
}

@Composable
fun HistoryNote.text(): String = when (this) {
    is HistoryNote.Message -> this.text
    HistoryNote.NotInNormalView -> stringResource(R.string.history_not_normal_view)
    HistoryNote.OnlyLastLines -> stringResource(R.string.history_only_last_lines, TranscriptPages.DUMP_LINES)
}

object TranscriptPages {
    const val DUMP_LINES = 1000

    fun rawLines(text: String): List<String> = text.trimEnd().split('\n').map { it.trimEnd('\r') }
    fun plain(raw: List<String>): List<String> = raw.map { Ansi.strip(it).trimEnd() }

    private fun isSeparator(line: String): Boolean {
        val t = line.trim()
        return t.length >= 10 && t.all { it == '─' || it == '━' }
    }

    /** Claude's transcript view says so in its last line. */
    fun inTranscript(plain: List<String>): Boolean =
        plain.lastOrNull { it.isNotBlank() }?.contains("ctrl+o to toggle") == true

    fun inTranscript(screenText: String): Boolean = inTranscript(plain(rawLines(screenText)))

    /** Lines above the fixed bottom part (separator lines, prompt box, status line). */
    fun contentEnd(plain: List<String>): Int {
        val from = (plain.size - 10).coerceAtLeast(0)
        val first = (from until plain.size).firstOrNull { isSeparator(plain[it]) } ?: return plain.size
        return first
    }

    /** Start of the newest dump in a scrollback read: the last Claude banner line. */
    fun dumpStart(plain: List<String>): Int? {
        val i = plain.indexOfLast { it.contains("Claude Code v") }
        return if (i >= 0) i else null
    }
}

/** Talks to Claude's transcript view of one pane. */
class TranscriptPager(private val core: Core, private val serverId: String, private val paneId: String) {
    private suspend fun screen(lines: Int = 300): String = core.readPane(serverId, paneId, lines, ansi = true).text
    private suspend fun keys(vararg k: String) = core.sendKeys(serverId, paneId, k.toList())

    /** Waits until the screen differs from [before] and then holds still; returns it (or the old one). */
    private suspend fun settle(before: String, timeoutMs: Long = 1_500): String {
        val end = System.currentTimeMillis() + timeoutMs
        var last = before
        var changed = false
        while (System.currentTimeMillis() < end) {
            delay(110)
            val now = screen()
            if (changed && now == last) return now
            if (now != before) changed = true
            last = now
        }
        return last
    }

    /** Opens the transcript view; null if Claude is not in its normal view. */
    private suspend fun open(): String? {
        val s0 = screen()
        if (TranscriptPages.inTranscript(s0)) return null
        // A menu (e.g. plan approval) is waiting for an answer: ctrl+o must not reach it.
        if (PaneTextTools.showsMenu(Ansi.strip(s0))) return null
        keys("ctrl+o")
        // settle() returns as soon as the screen changed and holds still; the long limit only
        // counts on a slow line, where a late ctrl+o would otherwise toggle back.
        val t = settle(s0, 4_000)
        return if (TranscriptPages.inTranscript(t)) t else { close(); null }
    }

    /** Back to the normal view; `ctrl+o` toggles, so only while the transcript is showing. */
    private suspend fun close() {
        repeat(2) {
            val s = screen()
            if (!TranscriptPages.inTranscript(s)) return
            keys("ctrl+o")
            settle(s, 3_000)
        }
    }

    /** Normal-view content (above the prompt box), to notice new output later. */
    suspend fun liveContent(): List<String> = liveContentOf(screen())

    fun liveContentOf(screenText: String): List<String> {
        val plain = TranscriptPages.plain(TranscriptPages.rawLines(screenText))
        return plain.take(TranscriptPages.contentEnd(plain))
    }

    /** `[` in the transcript view, then the last [TranscriptPages.DUMP_LINES] lines of scrollback. */
    suspend fun loadLast1000(h: ClaudeHistory) {
        if (h.busy) return
        h.busy = true; h.note = null
        try {
            val live = liveContent()
            val t = open() ?: run { h.note = HistoryNote.NotInNormalView; return }
            keys("[")
            settle(t, 4_000)
            val raw = TranscriptPages.rawLines(core.readPane(serverId, paneId, TranscriptPages.DUMP_LINES, ansi = true).text)
            val plain = TranscriptPages.plain(raw)
            val start = TranscriptPages.dumpStart(plain)
            val end = TranscriptPages.contentEnd(plain)
            val body = raw.subList(start ?: 0, end.coerceAtLeast(start ?: 0))
            h.clear()
            h.lines.addAll(body)
            // The dump ends with the conversation's end, which the live screen shows too.
            val older = PaneHistory.older(TranscriptPages.plain(body).joinToString("\n"), live.joinToString("\n")).size
            h.livePart = (body.size - older).coerceIn(0, body.size)
            h.liveAtCapture = live
            if (start == null) h.note = HistoryNote.OnlyLastLines
        } finally {
            // Back to the normal view even if the page goes away meanwhile (rotation, back):
            // a cancelled close() would leave Claude in its transcript view on the laptop.
            withContext(NonCancellable) { runCatching { withTimeout(10_000) { close() } } }
            h.busy = false
            h.lastAttempt = System.currentTimeMillis()
        }
    }
}
