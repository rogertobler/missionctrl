package ch.inscribe.missionctrl.ui

import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.graphics.ImageBitmap
import android.content.Context
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
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
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.platform.LocalContext
import android.util.Log
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.foundation.gestures.detectVerticalDragGestures
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
import ch.inscribe.missionctrl.SavedServer
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
import kotlinx.coroutines.withTimeoutOrNull

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
    /** The last picture with the opening it came from; one value, so the two always match. */
    private class Shown(val run: Int, val screen: TermScreen)
    private var shown by mutableStateOf<Shown?>(null)
    val screen: TermScreen? get() = shown?.screen
    /** Which opening of the terminal [screen] came from; a reconnect counts up. */
    val screenRun: Int get() = shown?.run ?: 0
    var problem by mutableStateOf<TermProblem?>(null); private set
    var takenOver by mutableStateOf(false); private set
    @Volatile private var runs = 0
    @Volatile var session: PaneTerminal? = null; private set

    private val view = core.pane(serverId, paneId)
    private val size = MutableStateFlow(0 to 0)
    // Changed on the main thread only (the page, the closing timer, leaving the app).
    private var run: Job? = null
    /** The last run closed: the next one starts after it has cleaned up. */
    private var stopping: Job? = null
    private var closing: Job? = null
    /**
     * herdr-core: a run is opening the session (also while the one before is still letting go,
     * and in the pause before a reconnect): input waits in [held] and goes into the session once
     * it is open, in the order typed.
     */
    private var opening = false
    private val held = ArrayList<Held>()
    private var heldBytes = 0
    private var heldKeys = 0
    /** Something did not fit in [held]: everything after it is refused too, so nothing arrives with a hole. */
    private var heldFull = false
    /** Counts input refused while held (one more per cut); the page says so. */
    var heldRefused by mutableIntStateOf(0); private set

    private sealed interface Held {
        class Bytes(val bytes: ByteArray) : Held
        class Keys(val names: List<String>) : Held
    }
    /**
     * The end of a session that was let go while still sending what was typed (herdr-core waits
     * for named keys before letting go). The next opening waits for it, so nothing typed there
     * can overtake it.
     */
    private var draining: CompletableDeferred<String?>? = null

    private fun onCore() = core.real.value.servers.any { it.id == serverId && it.backend == SavedServer.BACKEND_CORE }

    /** Input goes to the session, or waits while it opens (herdr-core); true when it is taken. */
    val accepting: Boolean get() = session != null || (opening && onCore())

    /**
     * A run is on (open, opening, or waiting to reopen): input belongs to it, never to another
     * connection that could overtake it.
     */
    val active: Boolean get() = run?.isActive == true

    /** Bytes typed into the terminal (main thread). */
    fun input(bytes: ByteArray) {
        session?.let { it.input(bytes); return }
        if (accepting && room(bytes.size, 0)) { held += Held.Bytes(bytes); heldBytes += bytes.size }
    }

    /** Key names for herdr-core to encode (main thread). */
    fun keys(names: List<String>) {
        session?.let { it.keys(names); return }
        if (accepting && room(0, 1)) { held += Held.Keys(names); heldKeys++ }
    }

    private fun room(bytes: Int, keys: Int): Boolean {
        if (heldFull) return false
        if (heldBytes + bytes <= HELD_MAX && heldKeys + keys <= HELD_KEYS_MAX) return true
        heldFull = true
        heldRefused++
        return false
    }

    /** The page is shown: open the session, or keep the one that is still open. */
    fun show() {
        closing?.cancel(); closing = null
        if (run?.isActive != true) {
            // A session just closed may still be letting go: its cleanup comes first.
            val before = stopping
            opening = true
            run = scope.launch(Dispatchers.Main.immediate) { before?.join(); loop() }
        }
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
        run?.let { it.cancel(); stopping = it }
        run = null
        opening = false
        dropHeld()
    }

    private fun dropHeld() { held.clear(); heldBytes = 0; heldKeys = 0; heldFull = false }

    /** After another device took the terminal over: take it back. */
    fun retake() { close(); show() }

    @OptIn(FlowPreview::class)
    private suspend fun loop() {
        takenOver = false
        draining?.let { old ->
            // The core ends a session it let go within a bound; only then may the next one type.
            if (withTimeoutOrNull(DRAIN_WAIT_MS) { old.await(); true } == null) {
                // Taking longer means something is wrong: what was typed meanwhile is not sent
                // later by surprise, and the wait ends at last even without word from the core.
                opening = false; dropHeld()
                problem = TermProblem.Lost
                if (withTimeoutOrNull(DRAIN_GIVE_UP_MS - DRAIN_WAIT_MS) { old.await(); true } == null) {
                    Log.w("LiveTerminal", "previous session never reported its end")
                }
            }
        }
        draining = null
        while (true) {
            opening = true
            val ended = CompletableDeferred<String?>()
            val run = ++runs
            val observer = object : PaneTerminalObserver {
                // Question detection and copying read the pane's plain text.
                // Called on a Rust thread: nothing may be thrown back across.
                override fun onScreen(screen: TermScreen) {
                    try {
                        // A late picture of an earlier opening is not shown.
                        if (run != runs) return
                        view.plain = TermText.plain(screen)
                        shown = Shown(run, screen)
                    } catch (e: Exception) {
                        Log.w("LiveTerminal", "screen not shown", e)
                    }
                }
                override fun onEnded(reason: String?) { ended.complete(reason) }
            }
            val (w, h) = size.first { it.first > 0 }
            val opened = runCatching { core.openTerminal(serverId, paneId, w, h, observer) }
                .onFailure {
                    if (it is CancellationException) {
                        // The opening stops too, and reports its end once it is over (it may
                        // have opened already and be letting go): the next run waits for that.
                        if (onCore()) draining = ended
                        throw it
                    }
                }
                .getOrElse {
                    // Typed into a terminal that did not open: not sent later by surprise.
                    opening = false; dropHeld()
                    problem = it.message?.let { m -> TermProblem.Reason(m) } ?: TermProblem.ConnectFailed; delay(3_000); continue
                }
            session = opened
            opening = false
            for (h in held) when (h) {
                is Held.Bytes -> opened.input(h.bytes)
                is Held.Keys -> opened.keys(h.names)
            }
            dropHeld()
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
                // Only herdr-core holds a session open to send what is still waiting.
                if (!ended.isCompleted && onCore()) draining = ended
            }
            if (reason == null) break
            if (reason.contains("taken over")) { takenOver = true; problem = TermProblem.TakenOver; break }
            problem = when {
                reason.startsWith("connection to the terminal lost") -> TermProblem.Lost
                reason.startsWith("terminal too large") -> TermProblem.TooLarge
                else -> TermProblem.Reason(reason)
            }
            // Typed during the pause goes into the next session.
            opening = true
            delay(2_000)
        }
    }
}

/**
 * Wait for a session that is still letting go before the pane opens again. herdr-core ends it at
 * the latest 15 s after letting go (`LEAVE_HARD` in the core); longer means something is wrong,
 * and the pane shows the connection as lost while it keeps waiting.
 */
private const val DRAIN_WAIT_MS = 20_000L
/** The core always reports a session's end; this only guards against it never coming. */
private const val DRAIN_GIVE_UP_MS = 60_000L
/** Most input held while the session opens (as much as herdr-core lets wait in a session). */
private const val HELD_MAX = 4 shl 20
private const val HELD_KEYS_MAX = 256

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

    fun send(bytes: ByteArray) { live?.input(bytes) }
    /** Key names herdr-core encodes, after the input sent before them (herdr-core only). */
    fun keys(names: List<String>) { live?.keys(names) }
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
    // The herdr it is connected to, so an Auto server on herdr-core counts too.
    val onCore = p.serverId in rememberCoreServers(core)
    // herdr-core: zoomed out from the cursor's part (1 = normal text).
    var zoom by remember(p.serverId, p.paneId) { mutableFloatStateOf(1f) }
    val pinchHintSeen by core.pinchHintSeen.collectAsState()
    // The chip's notes lie over the terminal, so they never change its rows.
    var explain by remember(p.serverId, p.paneId) { mutableStateOf(false) }
    var tookOver by remember(p.serverId, p.paneId) { mutableStateOf(false) }
    var lastWatching by remember(p.serverId, p.paneId) { mutableStateOf<Pair<Int, Boolean>?>(null) }
    val watching = if (onCore) live.screen?.watching else null
    val run = live.screenRun
    LaunchedEffect(run, watching) {
        // Only a change within one opening means the desktop took over; a reopening
        // after a lost link starts as a viewer again on its own.
        val before = lastWatching
        lastWatching = watching?.let { run to it }
        if (watching != true) { tookOver = false; return@LaunchedEffect }
        if (before != null && before.first == run && !before.second) {
            try { tookOver = true; delay(6_000) } finally { tookOver = false }
        }
    }

    // herdr refused a key from the bar or the keyboard: say so once per refusal.
    val context = LocalContext.current
    val keyErrors = live.screen?.keyErrors ?: 0uL
    var seenKeyErrors by remember(live) { mutableStateOf(keyErrors) }
    LaunchedEffect(run, keyErrors) {
        if (keyErrors > seenKeyErrors) context.toast(live.screen?.keyError ?: context.getString(R.string.common_error))
        seenKeyErrors = keyErrors
    }
    // Too much typed while input had to wait: the rest was not sent. Once per cut.
    val refused = (live.screen?.inputRefused ?: 0uL) + live.heldRefused.toULong()
    var seenRefused by remember(live) { mutableStateOf(refused) }
    LaunchedEffect(refused) {
        if (refused > seenRefused) context.toast(context.getString(R.string.common_error))
        seenRefused = refused
    }

    val measurer = rememberTextMeasurer(cacheSize = 256)
    val density = LocalDensity.current
    val base = remember(term.fontSize) { McType.output.copy(fontSize = term.fontSize.sp, lineHeight = TextUnit.Unspecified) }
    val cell = remember(base, density) {
        val m = measurer.measure("0".repeat(20), base, softWrap = false)
        Size(m.size.width / 20f, m.size.height.toFloat())
    }

    Column(Modifier.fillMaxSize()) {
    if (onCore) CoreChip(live.screen) { explain = !explain }
    BoxWithConstraints(
        Modifier.weight(1f).fillMaxWidth().padding(horizontal = 12.dp).clip(RoundedCornerShape(14.dp)).background(scheme.bg()),
    ) {
        val pad = 8.dp
        val cols = with(density) { ((maxWidth - pad * 2).toPx() / cell.width).toInt() }.coerceIn(10, 500)
        val rows = with(density) { ((maxHeight - pad * 2).toPx() / cell.height).toInt() }.coerceIn(4, 300)
        LaunchedEffect(live, cols, rows) { live.resize(cols, rows) }
        // Open while this page is shown and the app is on screen; afterwards it stays open for
        // the chosen time (Settings › Terminal), so swiping back is live at once.
        LaunchedEffect(live, active && onScreen) { if (active && onScreen) live.show() else live.hide(linger) }
        DisposableEffect(live) { onDispose { live.hide(linger) } }

        val description = stringResource(R.string.tui_terminal_desc, p.tab)
        val viewW = with(density) { (maxWidth - pad * 2).toPx() }
        val viewH = with(density) { (maxHeight - pad * 2).toPx() }
        val shown = live.screen
        // herdr-core while watching: the grid has the desktop's size. Larger than the view, it
        // shows the part around the cursor in normal text; pinching zooms out to the whole grid.
        // Everywhere else the grid is this view's size and nothing zooms.
        val minZoom = if (watching == true && shown != null) TermView.fitScale(shown.cols.toInt() * cell.width, shown.rows.toInt() * cell.height, viewW, viewH) else 1f
        val scale = zoom.coerceIn(minZoom, 1f)
        val pinchable = minZoom < 1f
        Canvas(
            Modifier.fillMaxSize().padding(pad).clipToBounds()
                // Two fingers zoom; seen first (Initial pass), so the scroll below lets go.
                .then(if (!pinchable) Modifier else Modifier.pointerInput(minZoom) {
                    awaitEachGesture {
                        do {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            if (event.changes.count { it.pressed } >= 2) {
                                zoom = (zoom.coerceIn(minZoom, 1f) * event.calculateZoom()).coerceIn(minZoom, 1f)
                                if (!pinchHintSeen) core.pinchHintSeen.set(true)
                                event.changes.forEach { it.consume() }
                            }
                        } while (event.changes.any { it.pressed })
                    }
                })
                // The keyboard opens only from the bar's keyboard button; a tap closes it.
                .pointerInput(link) { detectTapGestures(onTap = { link.hideKeyboard() }) }
                .pointerInput(link, cell) {
                    var drag = 0f
                    detectVerticalDragGestures(onDragEnd = { drag = 0f }, onDragCancel = { drag = 0f }) { _, dy ->
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
            val corner = TermView.offset(
                s.cols.toInt() * cell.width * scale, s.rows.toInt() * cell.height * scale,
                (s.cursorCol.toInt() + 0.5f) * cell.width * scale, (s.cursorRow.toInt() + 0.5f) * cell.height * scale,
                size.width, size.height,
            )
            translate(-corner.x, -corner.y) { scale(scale, Offset.Zero) {
            s.lines.forEachIndexed { row, line ->
                val y = row * cell.height
                for (run in line.runs) {
                    val (fg, bg) = TermText.colors(run, scheme)
                    val x = run.col.toInt() * cell.width
                    if (bg != null) drawRect(bg, Offset(x, y), Size(run.width.toInt() * cell.width, cell.height))
                    if (run.text.isNotBlank()) {
                        // Measured without the canvas's bounds: a run right of or below the view
                        // (zoomed, or moved to the cursor) must not be cut before it is scaled.
                        val text = measurer.measure(run.text, styleFor(base, run, fg), softWrap = false, maxLines = 1)
                        drawText(text, topLeft = Offset(x, y))
                    }
                }
            }
            if (s.cursorVisible) {
                val at = Offset(s.cursorCol.toInt() * cell.width, s.cursorRow.toInt() * cell.height)
                val cursor = scheme.fg()
                if (focused) drawRect(cursor.copy(alpha = 0.55f), at, cell)
                else drawRect(cursor.copy(alpha = 0.7f), at, cell, style = Stroke(width = 1.5f))
            }
            } }
            // An old picture under a problem is dimmed, so it does not look live.
            if (live.problem != null) drawRect(scheme.bg().copy(alpha = 0.75f))
        }
        if (shown != null && pinchable && scale > minZoom) {
            Overview(shown, cell, scale, viewW, viewH, Modifier.align(Alignment.TopEnd).padding(6.dp))
        }
        val showPinchHint = pinchable && !pinchHintSeen
        // Shown once: after a while it counts as seen, like after the first pinch.
        LaunchedEffect(showPinchHint) { if (showPinchHint) { delay(8_000); core.pinchHintSeen.set(true) } }
        val note = when {
            !onCore || shown == null -> null
            tookOver -> stringResource(R.string.tui_core_desktop_took_over)
            explain -> stringResource(if (shown.watching) R.string.tui_core_watching_hint else R.string.tui_core_typing_hint)
            showPinchHint -> stringResource(R.string.tui_core_zoom_hint)
            else -> null
        }
        // The note sits in the half away from the cursor, so it never covers the line being typed.
        val cursorAbove = shown != null && with(shown) {
            val corner = TermView.offset(
                cols.toInt() * cell.width * scale, rows.toInt() * cell.height * scale,
                (cursorCol.toInt() + 0.5f) * cell.width * scale, (cursorRow.toInt() + 0.5f) * cell.height * scale,
                viewW, viewH,
            )
            (cursorRow.toInt() + 0.5f) * cell.height * scale - corner.y < viewH / 2
        }
        if (note != null) {
            Text(
                note, style = McType.caption, color = if (tookOver) c.accentText else c.text,
                // Clear of the overview in the top corner.
                modifier = Modifier.align(if (cursorAbove) Alignment.BottomStart else Alignment.TopStart)
                    .padding(start = 8.dp, top = 8.dp, bottom = 8.dp, end = 8.dp + OVERVIEW_WIDTH + 12.dp).clip(RoundedCornerShape(8.dp)).background(c.inset)
                    .border(1.dp, c.line, RoundedCornerShape(8.dp))
                    .then(if (explain) Modifier.clickable { explain = false } else Modifier).padding(horizontal = 10.dp, vertical = 6.dp)
                    .semantics { liveRegion = LiveRegionMode.Polite },
            )
        }

        // Invisible view that owns the soft keyboard and hardware keys.
        AndroidView(
            factory = { ctx -> TermInputView(ctx).also { link.input = it } },
            update = { v ->
                // Enter sends, like the send button: close the keyboard if so set.
                v.onBytes = { link.send(it); if (hideAfterSend && TermKeys.hasEnter(it)) link.hideKeyboard() }
                v.onKeys = if (onCore) link::keys else null
                v.onFocus = { focused = it }
                // herdr-core only; the TUI path keeps plain cursor keys.
                v.appCursor = { onCore && live.screen?.appCursor == true }
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
        // Images the screen names: no room for them in the grid, so a button lists them.
        val shownScreen = live.screen
        val images = remember(shownScreen?.seq) { shownScreen?.let { ImagePaths.find(screenText(it)).asReversed() }.orEmpty() }
        if (images.isNotEmpty()) LiveImagesButton(core, p, images, Modifier.align(Alignment.TopEnd).padding(8.dp))
    }
    }
}

/**
 * The text of a live terminal, with the gaps between runs as spaces. A row that ends near the
 * right edge inside an image path runs on into the next: the terminal wrapped the path there.
 */
internal fun screenText(screen: TermScreen): String {
    val rows = screen.lines.map { line ->
        buildString { for (r in line.runs.sortedBy { it.col }) { while (length < r.col.toInt()) append(' '); append(r.text) } }.trimEnd()
    }
    val out = StringBuilder()
    rows.forEachIndexed { i, row ->
        val prev = rows.getOrNull(i - 1)
        // Checked against the line joined so far, so a path over three rows stays whole.
        val ahead = rows.subList(i, minOf(rows.size, i + 5)).joinToString("") { it.trimStart() }
        val wrapped = prev != null && prev.length >= screen.cols.toInt() - 8 && ImagePaths.brokenAcross(out.substring(out.lastIndexOf('\n') + 1), ahead)
        when {
            i == 0 -> out.append(row)
            wrapped -> out.append(row.trimStart())
            else -> out.append('\n').append(row)
        }
    }
    return out.toString()
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LiveImagesButton(core: Core, p: PaneRef, paths: List<String>, modifier: Modifier) {
    val c = LocalMc.current
    var open by remember { mutableStateOf(false) }
    var viewing by remember { mutableStateOf<Pair<ImageBitmap, String>?>(null) }
    val cwd = remember(p.serverId, p.paneId, paths) { core.paneCwd(p.serverId, p.paneId) }
    // Relative names count only once the file is found there: often they are just words in prose.
    // Kept across screen changes, so the count and an open sheet do not flicker.
    val found = remember(p.serverId, p.paneId, cwd) { mutableStateMapOf<String, Boolean>() }
    LaunchedEffect(paths, cwd) {
        // Again now and then: a file may appear later, or the connection come back.
        while (true) {
            core.awaitVisible() // quiet while the app is in the background
            for (path in paths.filter(ImagePaths::isRelative)) {
                found[path] = cwd != null && ImageCache.load(core, p.serverId, path, cwd).let { it != ImageState.Missing && it != ImageState.Failed }
            }
            delay(RECHECK_MS)
        }
    }
    // Absolute names first, so `./a.png` and `/dir/a.png` show once, under the full path.
    val shown = paths.sortedBy(ImagePaths::isRelative).distinctBy { ImagePaths.resolve(it, cwd) }
        .filter { !ImagePaths.isRelative(it) || found[it] == true }
    if (shown.isNotEmpty()) Box(
        modifier.clip(RoundedCornerShape(8.dp)).background(c.surface).border(1.dp, c.line, RoundedCornerShape(8.dp))
            .clickable(role = Role.Button) { open = true }.padding(horizontal = 10.dp, vertical = 6.dp),
    ) { Text(pluralStringResource(R.plurals.image_live_button, shown.size, shown.size), style = McType.small, color = c.text) }
    if (open && shown.isNotEmpty()) {
        ModalBottomSheet(onDismissRequest = { open = false }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = c.bg) {
            // Lazy, so only the images in view hold a bitmap.
            LazyColumn(Modifier.fillMaxWidth(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
                item { Text(stringResource(R.string.image_live_title), style = McType.title, color = c.text) }
                items(shown, key = { it }) { path ->
                    ImagePreview(core, p.serverId, path, cwd, showPath = true, onOpen = { viewing = it to path })
                }
            }
        }
    }
    viewing?.let { (bitmap, path) -> ImageViewer(bitmap, path, onDismiss = { viewing = null }) }
}

/**
 * herdr-core: whether the app only watches (the desktop's size) or types (this screen's size).
 * A tap explains (the note lies over the terminal).
 */
@Composable
private fun CoreChip(screen: TermScreen?, onExplain: () -> Unit) {
    val c = LocalMc.current
    if (screen == null) return
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onExplain)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        val (label, fg, bg) = if (screen.watching) Triple(stringResource(R.string.tui_core_watching), c.muted, c.inset)
            else Triple(stringResource(R.string.tui_core_typing), c.accentText, c.accentSoft)
        Pill(label, fg, bg, dot = !screen.watching)
        Text(
            stringResource(if (screen.watching) R.string.tui_core_size_desktop else R.string.tui_core_size_here, screen.cols.toInt(), screen.rows.toInt()),
            style = McType.caption, color = c.muted,
        )
        // A named key waits for herdr (it holds keys until typing pauses); input after it waits too.
        if (screen.keysPending) CircularProgressIndicator(Modifier.size(14.dp), color = c.muted, strokeWidth = 1.5.dp)
    }
}

private val OVERVIEW_WIDTH = 56.dp

/** Small map of the whole grid with the part on screen (only while zoomed in on a larger grid). */
@Composable
private fun Overview(s: TermScreen, cell: Size, scale: Float, viewW: Float, viewH: Float, modifier: Modifier) {
    val c = LocalMc.current
    val gridW = s.cols.toInt() * cell.width
    val gridH = s.rows.toInt() * cell.height
    val description = stringResource(R.string.tui_core_overview_desc)
    Canvas(modifier.size(OVERVIEW_WIDTH, OVERVIEW_WIDTH * (gridH / gridW).coerceIn(0.3f, 1.5f)).semantics { contentDescription = description }) {
        // One scale for both sides: a very tall or wide grid fits the map with room to spare
        // instead of running out of it.
        val k = minOf(size.width / gridW, size.height / gridH)
        val map = Size(gridW * k, gridH * k)
        drawRect(c.inset.copy(alpha = 0.85f), size = map)
        drawRect(c.muted, size = map, style = Stroke(1f))
        val at = TermView.offset(gridW * scale, gridH * scale, (s.cursorCol.toInt() + 0.5f) * cell.width * scale, (s.cursorRow.toInt() + 0.5f) * cell.height * scale, viewW, viewH)
        val x = (at.x / scale * k).coerceIn(0f, map.width)
        val y = (at.y / scale * k).coerceIn(0f, map.height)
        drawRect(c.accentText, Offset(x, y), Size((viewW / scale * k).coerceAtMost(map.width - x), (viewH / scale * k).coerceAtMost(map.height - y)), style = Stroke(1.5f))
    }
}

/** Pure geometry of the live terminal's view (tested on the JVM). */
object TermView {
    /** Smallest zoom that still shows the whole [gridW] × [gridH] grid; 1 when it fits. */
    fun fitScale(gridW: Float, gridH: Float, viewW: Float, viewH: Float): Float =
        if (gridW <= 0f || gridH <= 0f) 1f else minOf(1f, viewW / gridW, viewH / gridH)

    /**
     * Top-left of the visible part of a [contentW] × [contentH] picture in a [viewW] × [viewH]
     * view, keeping the cursor ([cursorX], [cursorY]) in sight: centred across, in the lower part
     * down (the newest lines are under the cursor). 0 where the picture fits.
     */
    fun offset(contentW: Float, contentH: Float, cursorX: Float, cursorY: Float, viewW: Float, viewH: Float): Offset {
        val x = if (contentW <= viewW) 0f else (cursorX - viewW / 2).coerceIn(0f, contentW - viewW)
        val y = if (contentH <= viewH) 0f else (cursorY - viewH * 0.7f).coerceIn(0f, contentH - viewH)
        return Offset(x, y)
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
    /** Keys whose modified forms have no legacy bytes of their own. */
    private val MOD_UNSAFE = setOf(KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_DEL, KeyEvent.KEYCODE_TAB, KeyEvent.KEYCODE_ESCAPE)
    private val APP_CURSOR = mapOf(
        KeyEvent.KEYCODE_DPAD_UP to 'A', KeyEvent.KEYCODE_DPAD_DOWN to 'B',
        KeyEvent.KEYCODE_DPAD_RIGHT to 'C', KeyEvent.KEYCODE_DPAD_LEFT to 'D',
        KeyEvent.KEYCODE_MOVE_HOME to 'H', KeyEvent.KEYCODE_MOVE_END to 'F',
    )

    /** xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl. */
    private fun mod(shift: Boolean, alt: Boolean, ctrl: Boolean) = 1 + (if (shift) 1 else 0) + (if (alt) 2 else 0) + (if (ctrl) 4 else 0)

    private fun csi(final: Char, m: Int) = if (m == 1) "$ESC[$final" else "$ESC[1;$m$final"
    private fun tilde(n: Int, m: Int) = if (m == 1) "$ESC[$n~" else "$ESC[$n;$m~"

    /**
     * Bytes for a key, or null when the key types nothing. [unicode] is the character the key
     * produces without Ctrl (0 for none). [appCursor]: the program asked for application cursor
     * keys (DECCKM), so plain arrows, Home and End go out as `ESC O x`.
     */
    fun bytes(keyCode: Int, unicode: Int, shift: Boolean = false, alt: Boolean = false, ctrl: Boolean = false, appCursor: Boolean = false): String? {
        val m = mod(shift, alt, ctrl)
        if (appCursor && m == 1) APP_CURSOR[keyCode]?.let { return "${ESC}O$it" }
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

    /** Whether [bytes] hold an Enter (CR) that sends, not one after ESC (Alt+Enter). */
    fun hasEnter(bytes: ByteArray): Boolean =
        bytes.indices.any { i -> bytes[i] == 13.toByte() && (i == 0 || bytes[i - 1] != 27.toByte()) }

    /**
     * Safe bytes for a modified Enter, Backspace or Esc on herdr-core, or null. herdr encodes
     * these chords as `CSI 27;m;k~` for a program that asked for no keyboard mode (a shell leaves
     * `;2;13~` on its line), and holds them until typing has paused for a moment. These bytes go
     * at once: Shift+Enter is LF (Ctrl+J), a new line in agent prompts and Enter in a shell, but
     * Ctrl+J in programs that bind it (nano justifies, fzf and vim move down), and herdr-core's
     * draft guard does not count it as the end of a typed line; Alt+ is ESC before the key;
     * Shift+Backspace is Backspace.
     */
    fun safeChord(keyCode: Int, shift: Boolean, alt: Boolean, ctrl: Boolean): String? {
        if (ctrl || shift == alt) return null
        return when (keyCode) {
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> if (shift) "\n" else "$ESC\r"
            KeyEvent.KEYCODE_DEL -> if (shift) "\u007F" else "$ESC\u007F"
            KeyEvent.KEYCODE_ESCAPE -> if (alt) "$ESC$ESC" else null
            else -> null
        }
    }

    /**
     * Bytes for a herdr key combo such as `enter`, `ctrl+c`, `shift+tab` or `ctrlalt+left`
     * (the key bar's names) on herdr-core, or null when it has no safe terminal bytes; the caller
     * then lets herdr encode the key for the program's keyboard mode. That is Enter, Backspace,
     * Tab and Esc with Ctrl, with Shift and Alt together, and Tab and Esc with modifiers other
     * than [safeChord]'s and Shift+Tab. Esc alone goes as its byte, at once: herdr would hold an
     * encoded one until typing has paused for a moment. [appCursor]: as in [bytes].
     */
    fun named(combo: String, appCursor: Boolean = false): String? {
        val parts = combo.split('+')
        val key = parts.last().ifEmpty { return null }
        var shift = false; var alt = false; var ctrl = false
        for (m in parts.dropLast(1)) when (m) {
            "shift" -> shift = true
            "alt" -> alt = true
            "ctrl" -> ctrl = true
            "ctrlalt" -> { ctrl = true; alt = true }
            else -> return null
        }
        val code = when (key) {
            "enter" -> KeyEvent.KEYCODE_ENTER
            "backspace" -> KeyEvent.KEYCODE_DEL
            "delete" -> KeyEvent.KEYCODE_FORWARD_DEL
            "tab" -> KeyEvent.KEYCODE_TAB
            "esc" -> KeyEvent.KEYCODE_ESCAPE
            "up" -> KeyEvent.KEYCODE_DPAD_UP
            "down" -> KeyEvent.KEYCODE_DPAD_DOWN
            "right" -> KeyEvent.KEYCODE_DPAD_RIGHT
            "left" -> KeyEvent.KEYCODE_DPAD_LEFT
            "home" -> KeyEvent.KEYCODE_MOVE_HOME
            "end" -> KeyEvent.KEYCODE_MOVE_END
            "pageup" -> KeyEvent.KEYCODE_PAGE_UP
            "pagedown" -> KeyEvent.KEYCODE_PAGE_DOWN
            else -> 0
        }
        if (code != 0) {
            val tabShift = code == KeyEvent.KEYCODE_TAB && shift && !alt && !ctrl
            safeChord(code, shift, alt, ctrl)?.let { return it }
            if ((shift || alt || ctrl) && code in MOD_UNSAFE && !tabShift) return null
            return bytes(code, 0, shift, alt, ctrl, appCursor)
        }
        var ch = when (key) {
            "space" -> ' '.code
            "plus" -> '+'.code
            else -> key.takeIf { it.codePointCount(0, it.length) == 1 }?.codePointAt(0) ?: return null
        }
        if (shift) {
            // Shift on a printable key: only a lone letter has a sure result (its capital).
            if (ctrl || alt || ch !in 'a'.code..'z'.code) return null
            ch = Character.toUpperCase(ch)
        }
        return bytes(0, ch, false, alt, ctrl)
    }

    /**
     * herdr's name for a hardware key chord without safe bytes (modified Enter, Backspace, Tab or
     * Esc, except Shift+Tab and the [safeChord]s), such as `ctrl+shift+enter`; null for any other key.
     */
    fun combo(keyCode: Int, shift: Boolean, alt: Boolean, ctrl: Boolean): String? {
        if (!(shift || alt || ctrl) || safeChord(keyCode, shift, alt, ctrl) != null) return null
        val name = when (keyCode) {
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> "enter"
            KeyEvent.KEYCODE_DEL -> "backspace"
            KeyEvent.KEYCODE_TAB -> if (shift && !alt && !ctrl) return null else "tab"
            KeyEvent.KEYCODE_ESCAPE -> "esc"
            else -> return null
        }
        return listOfNotNull("ctrl".takeIf { ctrl }, "alt".takeIf { alt }, "shift".takeIf { shift }, name).joinToString("+")
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
    /** herdr-core: chords without bytes of their own go by name; null elsewhere. */
    var onKeys: ((List<String>) -> Unit)? = null
    var onFocus: (Boolean) -> Unit = {}
    /** Whether the program wants application cursor keys right now. */
    var appCursor: () -> Boolean = { false }

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
        onKeys?.let { keys ->
            TermKeys.safeChord(event.keyCode, event.isShiftPressed, event.isAltPressed, event.isCtrlPressed)?.let { send(it); return }
            TermKeys.combo(event.keyCode, event.isShiftPressed, event.isAltPressed, event.isCtrlPressed)?.let { keys(listOf(it)); return }
        }
        val unicode = event.getUnicodeChar(event.metaState and (KeyEvent.META_CTRL_MASK or KeyEvent.META_ALT_MASK).inv())
        TermKeys.bytes(event.keyCode, unicode, event.isShiftPressed, event.isAltPressed, event.isCtrlPressed, appCursor())?.let(::send)
    }

    private fun send(s: String) = onBytes(s.toByteArray())
}
