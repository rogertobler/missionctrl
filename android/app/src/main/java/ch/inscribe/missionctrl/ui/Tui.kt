package ch.inscribe.missionctrl.ui

import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.PaneTerminal
import ch.inscribe.missionctrl.core.PaneTerminalObserver
import ch.inscribe.missionctrl.core.TermRun
import ch.inscribe.missionctrl.core.TermScreen
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/*
 * TUI mode: the pane as a real terminal. The core runs herdr's documented terminal-session
 * bridge (`herdr terminal session control`) over SSH and keeps the screen; this file draws it
 * cell by cell and turns the keyboard into terminal bytes. While a session is open the pane has
 * the phone's size; closing it (after the chosen time) or leaving the app gives it back.
 */

/** Why the terminal does not show: a reason from the server or the connection, or one of the app's own. */
sealed interface TermProblem {
    data class Reason(val text: String) : TermProblem
    data object ConnectFailed : TermProblem
    data object TakenOver : TermProblem
    /** The core's own reasons (its texts are English), shown in the app's language. */
    data object Lost : TermProblem
    data object TooLarge : TermProblem
}

/**
 * The live terminal of one pane. Core keeps it, so it outlives its page: the last image is there
 * at once on the next visit, and the session stays open for a while after the page leaves, so
 * swiping back and forth stays live. Leaving the app closes it at once.
 */
class LiveTerminal internal constructor(
    private val core: Core,
    private val scope: CoroutineScope,
    private val serverId: String,
    private val paneId: String,
) {
    var screen by mutableStateOf<TermScreen?>(null); private set
    var problem by mutableStateOf<TermProblem?>(null); private set
    var takenOver by mutableStateOf(false); private set
    @Volatile var session: PaneTerminal? = null; private set

    private val view = core.pane(serverId, paneId)
    private val size = MutableStateFlow(0 to 0)
    // Changed on the main thread only (the page, the closing timer, leaving the app).
    private var run: Job? = null
    private var closing: Job? = null

    /** The page is shown: open the session, or keep the one that is still open. */
    fun show() {
        closing?.cancel(); closing = null
        if (run?.isActive != true) run = scope.launch(Dispatchers.Main.immediate) { loop() }
    }

    /** The page has this size; an open session follows it. */
    fun resize(cols: Int, rows: Int) { size.value = cols to rows }

    /** The page left: close after [lingerMs], unless it is shown again before. */
    fun hide(lingerMs: Long) {
        if (run?.isActive != true) return
        closing?.cancel()
        closing = if (lingerMs <= 0) { close(); null } else scope.launch(Dispatchers.Main.immediate) { delay(lingerMs); close() }
    }

    /** Closes the session; the pane gets its size back. The last image stays. */
    fun close() {
        closing?.cancel(); closing = null
        run?.cancel(); run = null
    }

    /** After another device took the terminal over: take it back. */
    fun retake() { close(); show() }

    @OptIn(FlowPreview::class)
    private suspend fun loop() {
        takenOver = false
        while (true) {
            val ended = CompletableDeferred<String?>()
            val observer = object : PaneTerminalObserver {
                // Question detection and copying read the pane's plain text.
                override fun onScreen(screen: TermScreen) { view.plain = TermText.plain(screen); this@LiveTerminal.screen = screen }
                override fun onEnded(reason: String?) { ended.complete(reason) }
            }
            val (w, h) = size.first { it.first > 0 }
            val opened = runCatching { core.openTerminal(serverId, paneId, w, h, observer) }
                .onFailure { if (it is CancellationException) throw it }
                .getOrElse { problem = it.message?.let { m -> TermProblem.Reason(m) } ?: TermProblem.ConnectFailed; delay(3_000); continue }
            session = opened
            problem = null
            val reason = try {
                coroutineScope {
                    val follow = launch {
                        size.drop(1).debounce(150).collect { (cw, rh) -> opened.resize(cw.toUShort(), rh.toUShort()) }
                    }
                    ended.await().also { follow.cancel() }
                }
            } finally {
                session = null
                opened.release()
                opened.destroy()
            }
            if (reason == null) break
            if (reason.contains("taken over")) { takenOver = true; problem = TermProblem.TakenOver; break }
            problem = when {
                reason.startsWith("connection to the terminal lost") -> TermProblem.Lost
                reason.startsWith("terminal too large") -> TermProblem.TooLarge
                else -> TermProblem.Reason(reason)
            }
            delay(2_000)
        }
    }
}

/** The live terminals of all panes (one per pane, kept by Core). */
class LiveTerminals(private val core: Core, private val scope: CoroutineScope) {
    private val all = mutableMapOf<String, LiveTerminal>()

    init {
        // Leaving the app gives every pane its size back at once, whatever the setting.
        scope.launch(Dispatchers.Main.immediate) { core.onScreen.collect { on -> if (!on) all.values.forEach { it.close() } } }
    }

    fun of(serverId: String, paneId: String): LiveTerminal =
        all.getOrPut("$serverId|$paneId") { LiveTerminal(core, scope, serverId, paneId) }

    /** Drops the terminals of panes that are gone ("server|pane" keys), with their last picture. */
    fun retain(alive: Set<String>) {
        all.entries.removeAll { (key, live) -> (key !in alive).also { gone -> if (gone) live.close() } }
    }
}

/** Ties the bar of the detail to the live terminal of one page. */
class TuiLink {
    var live: LiveTerminal? = null
    val session: PaneTerminal? get() = live?.session
    internal var input: TermInputView? = null

    fun send(bytes: ByteArray) { session?.input(bytes) }
    fun send(text: String) = send(text.toByteArray())
    fun paste(text: String) = send(TermKeys.paste(text))

    fun showKeyboard() {
        val v = input ?: return
        v.requestFocus()
        v.imm().showSoftInput(v, InputMethodManager.SHOW_IMPLICIT)
    }

    fun hideKeyboard() {
        val v = input ?: return
        v.imm().hideSoftInputFromWindow(v.windowToken, 0)
        v.clearFocus()
    }
}

private fun View.imm() = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager

@Composable
fun TuiView(core: Core, p: PaneRef, active: Boolean, link: TuiLink) {
    val c = LocalMc.current
    val term by core.terminal.collectAsState()
    val scheme = term.scheme(c.dark)
    val onScreen by core.onScreen.collectAsState()
    val hideAfterSend by core.hideKeyboardAfterSend.collectAsState()
    val live = remember(p.serverId, p.paneId) { core.liveTerminals.of(p.serverId, p.paneId) }
    val linger by rememberUpdatedState(term.lingerSeconds * 1_000L)
    var focused by remember { mutableStateOf(false) }
    SideEffect { link.live = live }

    val measurer = rememberTextMeasurer(cacheSize = 256)
    val density = LocalDensity.current
    val base = remember(term.fontSize) { McType.output.copy(fontSize = term.fontSize.sp, lineHeight = TextUnit.Unspecified) }
    val cell = remember(base, density) {
        val m = measurer.measure("0".repeat(20), base, softWrap = false)
        Size(m.size.width / 20f, m.size.height.toFloat())
    }

    BoxWithConstraints(
        Modifier.fillMaxSize().padding(horizontal = 12.dp).clip(RoundedCornerShape(14.dp)).background(scheme.bg()),
    ) {
        val pad = 8.dp
        val cols = with(density) { ((maxWidth - pad * 2).toPx() / cell.width).toInt() }.coerceIn(10, 500)
        val rows = with(density) { ((maxHeight - pad * 2).toPx() / cell.height).toInt() }.coerceIn(4, 300)
        LaunchedEffect(live, cols, rows) { live.resize(cols, rows) }
        // Open while this page is shown and the app is on screen; afterwards it stays open for
        // the chosen time (Settings › Terminal), so swiping back is live at once.
        LaunchedEffect(live, active && onScreen) { if (active && onScreen) live.show() else live.hide(linger) }
        DisposableEffect(live) { onDispose { live.hide(linger) } }

        var drag by remember { mutableFloatStateOf(0f) }
        val description = stringResource(R.string.tui_terminal_desc, p.tab)
        Canvas(
            Modifier.fillMaxSize().padding(pad)
                // The keyboard opens only from the bar's keyboard button; a tap closes it.
                .pointerInput(link) { detectTapGestures(onTap = { link.hideKeyboard() }) }
                .pointerInput(link, cell) {
                    detectVerticalDragGestures(onDragEnd = { drag = 0f }) { _, dy ->
                        drag += dy
                        val lines = (drag / cell.height).toInt()
                        if (lines != 0) {
                            // Finger down shows older lines, like scrolling a list.
                            link.session?.scroll(lines > 0, kotlin.math.abs(lines).toUShort())
                            drag -= lines * cell.height
                        }
                    }
                }
                .semantics { contentDescription = description },
        ) {
            val s = live.screen ?: return@Canvas
            s.lines.forEachIndexed { row, line ->
                val y = row * cell.height
                for (run in line.runs) {
                    val (fg, bg) = TermText.colors(run, scheme)
                    val x = run.col.toInt() * cell.width
                    if (bg != null) drawRect(bg, Offset(x, y), Size(run.width.toInt() * cell.width, cell.height))
                    if (run.text.isNotBlank()) {
                        drawText(measurer, run.text, Offset(x, y), style = styleFor(base, run, fg), softWrap = false, maxLines = 1)
                    }
                }
            }
            if (s.cursorVisible) {
                val at = Offset(s.cursorCol.toInt() * cell.width, s.cursorRow.toInt() * cell.height)
                val cursor = scheme.fg()
                if (focused) drawRect(cursor.copy(alpha = 0.55f), at, cell)
                else drawRect(cursor.copy(alpha = 0.7f), at, cell, style = Stroke(width = 1.5f))
            }
            // An old picture under a problem is dimmed, so it does not look live.
            if (live.problem != null) drawRect(scheme.bg().copy(alpha = 0.75f))
        }

        // Invisible view that owns the soft keyboard and hardware keys.
        AndroidView(
            factory = { ctx -> TermInputView(ctx).also { link.input = it } },
            update = { v ->
                // Enter sends, like the send button: close the keyboard if so set.
                v.onBytes = { link.send(it); if (hideAfterSend && 13.toByte() in it) link.hideKeyboard() }
                v.onFocus = { focused = it }
                link.input = v
            },
            modifier = Modifier.size(1.dp).align(Alignment.TopStart),
        )

        // The last image stays, dimmed, while the session opens again; "connecting" shows only without one.
        val message = when (val problem = live.problem) {
            is TermProblem.Reason -> problem.text
            TermProblem.Lost -> stringResource(R.string.tui_connection_lost)
            TermProblem.TooLarge -> stringResource(R.string.tui_too_large)
            TermProblem.ConnectFailed -> stringResource(R.string.tui_connect_failed)
            TermProblem.TakenOver -> stringResource(R.string.tui_taken_over)
            null -> if (live.screen == null && active) stringResource(R.string.tui_connecting) else null
        }
        if (message != null) {
            // A card of its own: as plain text on the terminal picture it could not be read.
            Column(
                Modifier.align(Alignment.Center).padding(24.dp).clip(RoundedCornerShape(12.dp)).background(c.inset)
                    .border(1.dp, c.line, RoundedCornerShape(12.dp)).padding(horizontal = 16.dp, vertical = 12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(message, style = McType.small, color = c.text, textAlign = TextAlign.Center)
                if (live.takenOver) Box(
                    Modifier.clip(RoundedCornerShape(8.dp)).background(c.surface).clickable(role = Role.Button) { live.retake() }
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) { Text(stringResource(R.string.tui_take_back), style = McType.small, color = c.text) }
            }
        }
    }
}

private fun styleFor(base: TextStyle, run: TermRun, fg: Color): TextStyle = base.copy(
    color = fg,
    fontWeight = if (run.bold) FontWeight.Medium else null,
    fontStyle = if (run.italic) FontStyle.Italic else null,
    textDecoration = if (run.underline) TextDecoration.Underline else null,
)

/** Pure helpers for the TUI screen (tested on the JVM). */
object TermText {
    /** Colour from the core's encoding: -1 default, 0..255 palette, 0x1000000 | rgb true colour. */
    fun color(code: Int, scheme: TerminalScheme): Color? = when {
        code < 0 -> null
        code < 256 -> Ansi.xterm256(code, scheme)
        else -> Color(0xFF000000.toInt() or (code and 0xFFFFFF))
    }

    /** Foreground and background of a run (background null = the scheme's). */
    fun colors(run: TermRun, scheme: TerminalScheme): Pair<Color, Color?> {
        var fg = color(run.fg, scheme) ?: scheme.fg()
        var bg = color(run.bg, scheme)
        if (run.inverse) { val f = fg; fg = bg ?: scheme.bg(); bg = f }
        if (run.dim) fg = fg.copy(alpha = 0.6f)
        return fg to bg
    }

    /** The screen as plain text (question detection, copying), trailing blanks trimmed. */
    fun plain(screen: TermScreen): String = screen.lines.joinToString("\n") { line ->
        val b = StringBuilder()
        var at = 0
        for (run in line.runs) {
            while (at < run.col.toInt()) { b.append(' '); at++ }
            b.append(run.text)
            at = run.col.toInt() + run.width.toInt()
        }
        b.toString().trimEnd()
    }.trimEnd()
}

/** Keys as a terminal (xterm) sends them. */
object TermKeys {
    private const val ESC = "\u001B"

    /** xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl. */
    private fun mod(shift: Boolean, alt: Boolean, ctrl: Boolean) = 1 + (if (shift) 1 else 0) + (if (alt) 2 else 0) + (if (ctrl) 4 else 0)

    private fun csi(final: Char, m: Int) = if (m == 1) "$ESC[$final" else "$ESC[1;$m$final"
    private fun tilde(n: Int, m: Int) = if (m == 1) "$ESC[$n~" else "$ESC[$n;$m~"

    /**
     * Bytes for a key, or null when the key types nothing. [unicode] is the character the key
     * produces without Ctrl (0 for none).
     */
    fun bytes(keyCode: Int, unicode: Int, shift: Boolean = false, alt: Boolean = false, ctrl: Boolean = false): String? {
        val m = mod(shift, alt, ctrl)
        val special = when (keyCode) {
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> "\r"
            KeyEvent.KEYCODE_DEL -> "\u007F"
            KeyEvent.KEYCODE_FORWARD_DEL -> tilde(3, m)
            KeyEvent.KEYCODE_TAB -> if (shift) "$ESC[Z" else "\t"
            KeyEvent.KEYCODE_ESCAPE -> ESC
            KeyEvent.KEYCODE_DPAD_UP -> csi('A', m)
            KeyEvent.KEYCODE_DPAD_DOWN -> csi('B', m)
            KeyEvent.KEYCODE_DPAD_RIGHT -> csi('C', m)
            KeyEvent.KEYCODE_DPAD_LEFT -> csi('D', m)
            KeyEvent.KEYCODE_MOVE_HOME -> csi('H', m)
            KeyEvent.KEYCODE_MOVE_END -> csi('F', m)
            KeyEvent.KEYCODE_PAGE_UP -> tilde(5, m)
            KeyEvent.KEYCODE_PAGE_DOWN -> tilde(6, m)
            KeyEvent.KEYCODE_INSERT -> tilde(2, m)
            in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F4 -> {
                val f = "PQRS"[keyCode - KeyEvent.KEYCODE_F1]
                if (m == 1) "${ESC}O$f" else "$ESC[1;$m$f"
            }
            in KeyEvent.KEYCODE_F5..KeyEvent.KEYCODE_F12 -> tilde(intArrayOf(15, 17, 18, 19, 20, 21, 23, 24)[keyCode - KeyEvent.KEYCODE_F5], m)
            else -> null
        }
        if (special != null) return if (alt && special.length == 1) ESC + special else special
        if (unicode <= 0) return null
        var ch = unicode
        if (ctrl) {
            ch = when (unicode) {
                in 'a'.code..'z'.code -> unicode - 'a'.code + 1
                in '@'.code..'_'.code -> unicode - '@'.code
                ' '.code, '2'.code -> 0
                '/'.code -> 0x1F
                '?'.code -> 0x7F
                else -> unicode
            }
        }
        val text = String(Character.toChars(ch))
        return if (alt) ESC + text else text
    }

    /** What the soft keyboard commits: Enter as CR, like a terminal sends it. */
    fun text(committed: CharSequence): String = committed.toString().replace("\r\n", "\r").replace('\n', '\r')

    /** Text as one bracketed paste, so a shell or editor takes it as pasted, not typed. */
    fun paste(text: String) = "$ESC[200~$text$ESC[201~"

    /**
     * A message from our text field: several lines go as one paste, else the agent would take the
     * first line break as Enter. A break at the end would only leave an empty line.
     */
    fun message(text: String): String = text.trimEnd('\n').let { if ('\n' in it) paste(it) else it }
}

/**
 * Invisible editor that receives the soft keyboard and hardware keys and hands them over as
 * terminal bytes. Like a password field: no suggestions, nothing learned.
 */
class TermInputView(context: Context) : View(context) {
    var onBytes: (ByteArray) -> Unit = {}
    var onFocus: (Boolean) -> Unit = {}

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: android.graphics.Rect?) {
        super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
        onFocus(gainFocus)
    }

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or EditorInfo.IME_ACTION_NONE
        return object : BaseInputConnection(this, true) {
            private fun flush() {
                val e = editable ?: return
                if (e.isNotEmpty()) { send(TermKeys.text(e)); e.clear() }
            }

            override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
                super.commitText(text, newCursorPosition)
                flush()
                return true
            }

            override fun finishComposingText(): Boolean {
                super.finishComposingText()
                flush()
                return true
            }

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                val e = editable
                if (e.isNullOrEmpty()) repeat(beforeLength.coerceAtLeast(1)) { send("\u007F") }
                else super.deleteSurroundingText(beforeLength, afterLength)
                return true
            }

            override fun sendKeyEvent(event: KeyEvent): Boolean {
                if (event.action == KeyEvent.ACTION_DOWN) handleKey(event)
                return true
            }
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean =
        if (keyCode == KeyEvent.KEYCODE_BACK) super.onKeyDown(keyCode, event) else { handleKey(event); true }

    private fun handleKey(event: KeyEvent) {
        val unicode = event.getUnicodeChar(event.metaState and (KeyEvent.META_CTRL_MASK or KeyEvent.META_ALT_MASK).inv())
        TermKeys.bytes(event.keyCode, unicode, event.isShiftPressed, event.isAltPressed, event.isCtrlPressed)?.let(::send)
    }

    private fun send(s: String) = onBytes(s.toByteArray())
}
