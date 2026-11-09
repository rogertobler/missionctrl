package ch.inscribe.missionctrl.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextIndent
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.DemoFleet
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.Pin
import ch.inscribe.missionctrl.core.Status
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DetailScreen(core: Core, nav: Nav, target: Screen.Detail, showBack: Boolean) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val state by core.state.collectAsState()
    val grouped by core.groupByWorkspace.collectAsState()
    val barIds by core.detailBar.collectAsState()
    val pins by core.pins.collectAsState()
    // The demo has no live terminal: its sample panes always show in the normal view.
    val tui = core.terminal.collectAsState().value.tui && !core.demo.collectAsState().value
    // TUI mode: the bar of a page talks to that page's live terminal.
    val tuiLinks = remember { mutableMapOf<String, TuiLink>() }
    fun linkOf(key: String) = tuiLinks.getOrPut(key) { TuiLink() }

    // Page order stays as it was when the detail opened: a sent message changes the agent's
    // status and with it the urgency order, which must not move the pages under the finger.
    val order = remember(grouped) { StablePageOrder() }
    val pages = remember(state, grouped, target) { order.apply(detailPages(state, grouped, target)) }
    val currentPages by rememberUpdatedState(pages)
    // A tab or workspace just created is opened before the fleet state knows it. Wait for it,
    // however long it takes: never show another tab instead. On a slow line that once opened
    // someone else's first agent and loaded its history there.
    val known = pages.any { it.serverId == target.serverId && it.paneId == target.paneId }
    var slow by remember(target) { mutableStateOf(false) }
    LaunchedEffect(target) { delay(5_000); slow = true }
    if (!known) {
        Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(stringResource(if (slow) R.string.detail_not_there else R.string.detail_opening), style = McType.small, color = c.muted, textAlign = TextAlign.Center)
            if (slow) SecondaryButton(stringResource(R.string.detail_back)) { nav.back() }
        }
        return
    }
    if (pages.isEmpty()) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(stringResource(R.string.detail_no_tab), style = McType.small, color = c.muted) }
        return
    }
    val startIndex = pages.indexOfFirst { it.serverId == target.serverId && it.paneId == target.paneId }.coerceAtLeast(0)
    val pager = rememberPagerState(initialPage = startIndex) { pages.size }
    LaunchedEffect(target) { if (pager.currentPage != startIndex) pager.scrollToPage(startIndex) }
    LaunchedEffect(pager) {
        snapshotFlow { pager.settledPage }.collect { i ->
            currentPages.getOrNull(i)?.let { p ->
                val d = Screen.Detail(p.serverId, p.paneId)
                nav.lastDetail = d
                if (nav.current is Screen.Detail && nav.current != d) nav.stack[nav.stack.lastIndex] = d
            }
        }
    }
    val page = pages[pager.currentPage.coerceIn(0, pages.lastIndex)]
    val agent = state.agents.firstOrNull { it.serverId == page.serverId && it.paneId == page.paneId }
    val pinned = pins.any { it.serverId == page.serverId && it.tabId == page.tabId }

    val pageKey = page.serverId + "|" + page.paneId
    // Keys typed after a swipe must not go to the page that was left.
    LaunchedEffect(pageKey) { tuiLinks.forEach { (k, l) -> if (k != pageKey) l.hideKeyboard() } }
    var switcher by remember { mutableStateOf(false) }
    var tabOptions by remember { mutableStateOf<TabLine?>(null) }
    var mode by rememberSaveable(pageKey) { mutableStateOf(Entry.BAR) }
    // Drafts live in Core: they survive folding the phone and swiping to another agent and back.
    val draft = core.drafts[pageKey].orEmpty()
    fun setDraft(value: String, key: String = pageKey) { if (value.isEmpty()) core.drafts.remove(key) else core.drafts[key] = value }
    var uploading by remember { mutableStateOf(false) }
    val listStates = remember { mutableStateMapOf<String, LazyListState>() }
    fun toast(msg: String?) = context.toast(msg ?: resources.getString(R.string.common_error))

    val speech by core.speech.collectAsState()
    // What is said goes to the agent it was said to, even if the pager moved meanwhile.
    var voiceTarget by remember { mutableStateOf<PaneRef?>(null) }
    val (voice, startVoice) = rememberVoice(speech) { heard, complete ->
        val t = voiceTarget ?: page
        val tKey = t.serverId + "|" + t.paneId
        val existing = core.drafts[tKey].orEmpty()
        if (complete && speech.sendsHeard(existing)) {
            scope.launch { runCatching { core.sendText(t.serverId, t.paneId, heard, enter = true) }.onFailure { toast(it.message) } }
        } else { setDraft((existing + " " + heard).trim(), tKey); if (tKey == pageKey && mode != Entry.TYPING) mode = Entry.HEARD }
    }
    fun listen() { voiceTarget = page; startVoice() }
    val hideKeyboard = rememberHideKeyboard()
    val hideKbAfterSend by core.hideKeyboardAfterSend.collectAsState()
    val suggestions by core.keyboardSuggestions.collectAsState()
    val dock by core.showDock.collectAsState()
    val menuRight by core.menuRight.collectAsState()
    var allKeys by remember { mutableStateOf(false) }
    /** Ctrl/Alt waiting for the next key; a swipe to another tab drops it. */
    var mod by remember(pageKey) { mutableStateOf<String?>(null) }
    var modFocus by remember { mutableIntStateOf(0) }
    fun sendDraft() {
        // Read now, not from the last frame: the keyboard may commit a correction just before Send.
        val text = core.drafts[pageKey].orEmpty()
        if (text.isBlank()) return
        // Emptied right away, so a second Send (double tap) has nothing to send.
        setDraft("")
        scope.launch {
            // A waiting agent's menu may take the text ("Type something.", "Chat about this"): Core routes it.
            val waiting = core.state.value.agents.any { it.serverId == page.serverId && it.paneId == page.paneId && it.status == Status.BLOCKED }
            runCatching { if (waiting) core.answerText(page.serverId, page.paneId, text) else core.sendText(page.serverId, page.paneId, text, enter = true) }
                .onSuccess {
                    // Typing may go on with the keyboard open, if so set; speech always returns to the bar.
                    if (mode != Entry.TYPING || hideKbAfterSend) { mode = Entry.BAR; hideKeyboard() }
                }.onFailure { setDraft(text + core.drafts[pageKey].orEmpty()); toast(it.message) }
        }
    }
    fun uploadAndInsert(name: String, bytes: ByteArray) = scope.launch {
        uploading = true
        runCatching { core.upload(page.serverId, name, bytes) }
            .onSuccess { path ->
                if (tui) linkOf(pageKey).send("$path ")
                else { setDraft((core.drafts[pageKey].orEmpty() + " " + path + " ").trimStart()); mode = Entry.TYPING }
                toast(resources.getString(R.string.detail_uploaded, name))
            }
            .onFailure { toast(it.message) }
        uploading = false
    }
    val pickFile = rememberFilePicker { name, bytes -> uploadAndInsert(name, bytes) }
    val switchDescription = stringResource(R.string.detail_switch_agent_desc, page.workspace)
    val toEndDescription = stringResource(R.string.detail_to_end)

    Column(Modifier.fillMaxSize()) {
        // Header
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 8.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            if (showBack) IconAction(McIcons.Back, stringResource(R.string.detail_back), c.text) { nav.back() } else Box(Modifier.width(12.dp))
            WorkspaceMark(core, page.serverId, page.workspaceId, page.workspace, 26.dp, tab = page.tab)
            AgentGlyph(core, page.agentType, page.customName, modifier = Modifier.padding(start = 6.dp), tab = TabRef(page.serverId, page.workspaceId, page.workspace, page.tab))
            Column(
                Modifier.weight(1f).heightIn(min = 44.dp).clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { switcher = true }
                    .semantics { contentDescription = switchDescription }.padding(start = 8.dp),
                verticalArrangement = Arrangement.Center,
            ) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    EllipsisText(tabTitleOf(core, page.tab, page.workspace, page.workspace), McType.bodyStrong, c.text)
                    McIcon(McIcons.ChevronDown, c.muted, 14.dp)
                }
                EllipsisText(agent?.let { pathLineOf(core, it) } ?: "${page.serverName}/${page.workspace}", McType.path, c.muted)
            }
            agent?.let {
                val waiting = it.status == Status.BLOCKED
                Pill(statusText(it), if (waiting) c.accentText else c.muted, if (waiting) c.accentSoft else c.inset)
            }
            IconAction(if (pinned) McIcons.PinFilled else McIcons.Pin, if (pinned) stringResource(R.string.common_unpin) else stringResource(R.string.detail_pin), if (pinned) c.accentText else c.muted) {
                core.togglePin(Pin(page.serverId, page.tabId, page.workspace, page.tab))
            }
        }
        // No position dots; swiping and the title switcher are enough.
        // Output pages
        val listState = listStates[page.serverId + page.paneId]
        val scrolledUp by remember(listState) { derivedStateOf { (listState?.firstVisibleItemIndex ?: 0) > 0 } }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            HorizontalPager(pager, Modifier.fillMaxSize(), userScrollEnabled = !voice.listening, key = { pages[it].serverId + pages[it].paneId }) { i ->
                val p = pages[i]
                val listState = rememberLazyListState()
                SideEffect { listStates[p.serverId + p.paneId] = listState }
                val status = state.agents.firstOrNull { it.serverId == p.serverId && it.paneId == p.paneId }?.status
                if (tui) TuiView(core, p, active = i == pager.currentPage, link = linkOf(p.serverId + "|" + p.paneId))
                else OutputView(core, p, status, active = i == pager.currentPage, listState = listState)
            }
            // Floats over the output instead of joining the bar, so the bar's buttons never shift.
            if (scrolledUp && !tui) Box(
                Modifier.align(Alignment.BottomEnd).padding(end = 22.dp, bottom = 10.dp).size(44.dp).clip(RoundedCornerShape(22.dp))
                    .background(c.surface).clickable(role = Role.Button) { scope.launch { listState?.animateScrollToItem(0) } }
                    .semantics { contentDescription = toEndDescription },
                contentAlignment = Alignment.Center,
            ) { McIcon(McIcons.ToEnd, c.text, 18.dp) }
        }
        // Toolbar / input
        Box(Modifier.padding(horizontal = 12.dp, vertical = 10.dp).clip(RoundedCornerShape(14.dp))) {
            when {
                // Keys would land in Claude's transcript view while the history is being paged.
                core.paneOrNull(page.serverId, page.paneId)?.claude?.busy == true -> Box(
                    Modifier.fillMaxWidth().height(48.dp).background(c.surface).padding(horizontal = 14.dp), contentAlignment = Alignment.CenterStart,
                ) { Text(stringResource(R.string.detail_history_busy), style = McType.small, color = c.muted) }
                // The draft of the pane spoken to (voiceTarget), where the text will land.
                voice.listening -> ListeningEntry((voiceTarget ?: page).let { core.drafts[it.serverId + "|" + it.paneId] }.orEmpty(), voice)
                // One branch for both, so a tap into heard text keeps the field and opens the keyboard.
                // Also in the TUI: text is written and corrected here and goes to the pane on send.
                mode.showsField(draft) -> EntryBar(
                    draft, { setDraft(it) }, mode, { mode = it }, suggestions, onSpeak = { listen() }, onSend = { sendDraft() },
                    // The draft stays and is back with the next ⌨, like after swiping to another agent.
                    onRaw = if (tui) { { mode = Entry.BAR; linkOf(pageKey).showKeyboard() } } else null,
                )
                else -> {
                    val working = agent?.status == Status.WORKING
                    val view = core.pane(page.serverId, page.paneId)
                    val blocked = agent?.status == Status.BLOCKED
                    val kind = remember(view.plain, blocked) { if (blocked) PaneTextTools.kind(PaneTextTools.question(view.plain)) else QuestionKind.UNKNOWN }
                    val menuNow = remember(view.plain, blocked) { if (blocked) PaneTextTools.menu(PaneTextTools.question(view.plain)) else null }
                    val keys = PaneTextTools.fitting(barIds, kind).filter { it.id != "stop" || working }
                    fun sendKeys(keys: List<String>) = scope.launch { runCatching { core.sendKeys(page.serverId, page.paneId, keys) }.onFailure { toast(it.message) } }
                    fun press(k: QuickKey) {
                        // A modifier arms (tapped again: off); armed, a plain key goes out with it.
                        k.mod?.let { m -> mod = if (mod == m) null else m; return }
                        val armed = mod
                        // The keyboard button opens the keyboard for the armed key instead.
                        if (armed != null && k.id == "kb") { modFocus++; return }
                        mod = null
                        if (armed != null && k.keys.size == 1) { hideKeyboard(); sendKeys(listOf(ModKeys.chord(armed, k.keys[0]))); return }
                        when (k.id) {
                            "mic" -> listen()
                            "kb" -> mode = Entry.TYPING
                            "copy" -> {
                                Attachments.copy(context, resources.getString(R.string.detail_clip_label), PaneTextTools.lastAnswer(view.text())); toast(resources.getString(R.string.detail_copied))
                            }
                            "paste" -> when (val clip = Attachments.clipboard(context)) {
                                is ClipContent.Text -> if (tui) linkOf(pageKey).paste(clip.text) else { setDraft(draft + clip.text); mode = Entry.TYPING }
                                is ClipContent.File -> scope.launch {
                                    runCatching { Attachments.read(context, clip.uri, clip.mime) }
                                        .onSuccess { (n, b) -> uploadAndInsert(n, b) }.onFailure { toast(it.message) }
                                }
                                ClipContent.Empty -> toast(resources.getString(R.string.detail_clipboard_empty))
                            }
                            "file" -> pickFile()
                            else -> {
                                // ↓/↑ onto "Type something." or "Chat about this" opens the field right away.
                                if (PaneTextTools.landsOnText(menuNow, k.keys.singleOrNull())) mode = Entry.TYPING
                                sendKeys(k.keys)
                            }
                        }
                    }
                    // ☰ (without the dock) first or last; ⋯ for all other keys at the end of the own keys.
                    val menu = if (!dock) BarButton("menu", stringResource(R.string.common_menu), McIcons.Menu) { nav.menuOpen = true } else null
                    val more = BarButton("more", stringResource(R.string.allkeys_title), McIcons.More) { allKeys = true }
                    val own = keys.map { k -> BarButton(k.id, stringResource(k.description), k.icon, if (k.icon == null || k.id == "stop") stringResource(k.label) else null) { press(k) } }
                    Column(Modifier.fillMaxWidth().background(c.line), verticalArrangement = Arrangement.spacedBy(1.dp)) {
                        mod?.let { m ->
                            ModBar(m, onReopen = { modFocus++ }) { mod = null; hideKeyboard() }
                            ModCapture(modFocus) { key -> mod = null; hideKeyboard(); sendKeys(listOf(ModKeys.chord(m, key))) }
                        }
                        ButtonBar(if (menuRight) own + more + listOfNotNull(menu) else listOfNotNull(menu) + own + more, wide = setOf("stop"))
                    }
                    if (allKeys) AllKeysSheet(QuickKeys.availableDetail, barIds, ::press, onAdd = { k -> addToBar(core, "detail", k, context) }, onDismiss = { allKeys = false })
                }
            }
        }
        if (uploading) Text(stringResource(R.string.detail_uploading), style = McType.caption, color = c.muted, modifier = Modifier.padding(start = 16.dp, bottom = 6.dp))
    }

    if (switcher) {
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(onDismissRequest = { switcher = false }, sheetState = sheet, containerColor = c.bg) {
            // Long press: the tab's options, as on the start page and the workspaces.
            Switcher(core, nav, current = page, onOptions = { switcher = false; tabOptions = it }, onDone = { switcher = false })
        }
    }
    tabOptions?.let { line -> TabSheet(core, nav, line) { tabOptions = null } }
}

/** Keeps pages where they first appeared; new ones go to the end, closed ones drop out. */
private class StablePageOrder {
    private val keys = mutableListOf<String>()
    fun apply(fresh: List<PaneRef>): List<PaneRef> {
        val byKey = fresh.associateBy { it.serverId + "|" + it.paneId }
        keys.retainAll(byKey.keys)
        byKey.keys.forEach { if (it !in keys) keys.add(it) }
        return keys.map { byKey.getValue(it) }
    }
}

@Composable
private fun OutputView(core: Core, p: PaneRef, status: Status?, active: Boolean, listState: LazyListState) {
    val c = LocalMc.current
    val term by core.terminal.collectAsState()
    val scheme = term.scheme(c.dark)
    val scope = rememberCoroutineScope()
    val view = remember(p.serverId, p.paneId) { core.pane(p.serverId, p.paneId) }
    val resting = status == Status.IDLE || status == Status.DONE
    // The polling loop below outlives status changes: it reads the current value through this.
    val restingNow by rememberUpdatedState(resting)
    // Fullscreen Claude: coloured history through its transcript view (History.kt); the demo has none.
    val claude = p.agentType?.contains("claude", ignoreCase = true) == true && p.serverId != DemoFleet.SERVER
    val history = view.claude
    val pager = remember(p.serverId, p.paneId) { TranscriptPager(core, p.serverId, p.paneId) }
    fun load() {
        if (!claude || !restingNow || history.busy) return
        if (System.currentTimeMillis() - history.lastAttempt < 1_500) return
        scope.launch { runCatching { pager.loadLast1000(history) }.onFailure { history.note = it.message?.let { m -> HistoryNote.Message(m) } } }
    }
    // A resting agent's full history can only be harvested then; ask again whenever it comes to rest.
    LaunchedEffect(resting) { if (resting) view.harvestWanted = true }
    val nearTop by remember(listState) {
        derivedStateOf { listState.layoutInfo.totalItemsCount > 0 && (listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0) >= listState.layoutInfo.totalItemsCount - 3 }
    }
    LaunchedEffect(nearTop, resting, active) { if (nearTop && active && !claude) view.harvestWanted = true }
    val tick by core.inputTick.collectAsState()
    LaunchedEffect(p.serverId, p.paneId, active, tick) {
        if (tick > 0) delay(150) // let the agent redraw after a key
        while (active) {
            core.awaitVisible() // nothing to show while the app is not on screen
            // While paging, the screen shows Claude's transcript view, not the live one.
            if (history.busy) { delay(300); continue }
            val screen = runCatching { core.readPane(p.serverId, p.paneId, PaneHistory.LINES, ansi = true) }.getOrNull()
            // herdr 0.9.5 answers pane.read with revision 0 every time (pane get counts up): compare
            // the text too, else the normal view stays at its first picture.
            if (screen != null && !history.busy && (screen.revision.toLong() != view.revision || screen.text != view.ansi)) {
                val plain = withContext(Dispatchers.Default) { Ansi.strip(screen.text) }
                view.ansi = screen.text; view.plain = plain; view.revision = screen.revision.toLong()
            }
            val now = System.currentTimeMillis()
            if (screen != null && claude) {
                // New output since the history was taken: start over from the bottom.
                if (restingNow && !history.busy && history.lines.isNotEmpty() && now - history.lastAttempt > PaneHistory.MIN_INTERVAL_MS &&
                    pager.liveContentOf(screen.text) != history.liveAtCapture) { history.clear(); load() }
                if (restingNow && history.lines.isEmpty() && history.lastAttempt == 0L) load()
            } else if (screen != null && view.harvestWanted && restingNow && now - view.lastHarvest > PaneHistory.MIN_INTERVAL_MS) {
                view.harvestWanted = false
                view.lastHarvest = now
                runCatching { core.readPane(p.serverId, p.paneId, PaneHistory.LINES) }.onSuccess { plain ->
                    val older = withContext(Dispatchers.Default) { PaneHistory.older(plain.text, view.plain) }
                    if (older.isNotEmpty()) view.olderPlain = older
                }
            }
            delay(1_200)
        }
    }
    val raw = view.ansi.orEmpty()
    val plainHistory = view.olderPlain
    val colouredHistory = if (claude) history.older() else emptyList()
    val style = McType.output.copy(fontSize = term.fontSize.sp, lineHeight = (term.fontSize * 1.6f).sp)
    // Width of one monospaced column, to know how many fit and to indent wrapped lines (Reflow).
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val charPx = remember(style, density) { measurer.measure("0".repeat(20), style).size.width / 20f }
    val charSp = charPx / (density.density * density.fontScale)
    BoxWithConstraints(Modifier.fillMaxSize().padding(horizontal = 12.dp).clip(RoundedCornerShape(14.dp)).background(scheme.bg())) {
        val cols = with(density) { ((maxWidth - 28.dp).toPx() / charPx).toInt() }
        // Colours and line fitting off the main thread; the old lines stay until the new ones are ready.
        val display by produceState(emptyList<DisplayLine>(), raw, plainHistory, colouredHistory, scheme, cols) {
            value = withContext(Dispatchers.Default) {
                val older = if (claude) Ansi.lines(colouredHistory.joinToString("\n"), scheme).take(colouredHistory.size)
                    else plainHistory.map { AnnotatedString(it) }
                val lines = (older + Ansi.lines(raw.trimEnd(), scheme)).map { l -> if (l.text.startsWith("│") || l.text.startsWith("┃")) l.subSequence(1, l.length) else l }
                Reflow.layout(lines, cols).reversed()
            }
        }
        SelectionContainer {
            LazyColumn(state = listState, reverseLayout = true, contentPadding = PaddingValues(14.dp), modifier = Modifier.fillMaxSize()) {
                items(display.size) { i ->
                    val d = display[i]
                    Text(
                        if (d.text.isEmpty()) AnnotatedString(" ") else d.text,
                        style = if (d.hang > 0) style.copy(textIndent = TextIndent(restLine = (d.hang * charSp).sp)) else style,
                        color = scheme.fg(), softWrap = !d.noWrap, maxLines = if (d.noWrap) 1 else Int.MAX_VALUE,
                        overflow = TextOverflow.Clip,
                    )
                }
                // Only once there are lines: alone in the first frame, the list took this keyed item
                // as its anchor and followed it to the top when the lines came (a Claude tab longer
                // than the screen opened at its start, not its end).
                if (claude && display.isNotEmpty()) item(key = "history-top") {
                    HistoryTop(history, resting, scheme.fg().copy(alpha = 0.6f), onLoad1000 = {
                        history.lastAttempt = 0L; load()
                    })
                }
            }
        }
    }
}

/** Top of a Claude history: what is loading, why nothing more comes, and the 1000-line button. */
@Composable
private fun HistoryTop(h: ClaudeHistory, resting: Boolean, tint: Color, onLoad1000: () -> Unit) {
    val c = LocalMc.current
    Column(Modifier.fillMaxWidth().padding(bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val note = h.note
        val text = when {
            h.busy -> stringResource(R.string.detail_history_loading, TranscriptPages.DUMP_LINES)
            note != null -> note.text()
            h.lines.isNotEmpty() -> stringResource(R.string.detail_history_start)
            !resting -> stringResource(R.string.detail_history_after_rest)
            else -> stringResource(R.string.detail_history_not_loaded)
        }
        Text(text, style = McType.caption, color = tint)
        if (resting && !h.busy) {
            Box(
                Modifier.clip(RoundedCornerShape(8.dp)).background(c.inset).clickable(role = Role.Button, onClick = onLoad1000)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
            ) { Text(if (h.lines.isEmpty()) stringResource(R.string.detail_history_fetch, TranscriptPages.DUMP_LINES) else stringResource(R.string.detail_history_reload), style = McType.small, color = c.text) }
        }
    }
}

@Composable
private fun Switcher(core: Core, nav: Nav, current: PaneRef, onOptions: (TabLine) -> Unit, onDone: () -> Unit) {
    val c = LocalMc.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by core.state.collectAsState()
    val grouped by core.groupByWorkspace.collectAsState()
    var newTabIn by remember { mutableStateOf<Pair<String, String>?>(null) }
    var confirm by remember { mutableStateOf<Triple<String, String, String>?>(null) } // server, tab, label
    val options = stringResource(R.string.mission_options)

    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 12.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val target = newTabIn
        if (target != null) {
            NewTabForm(core, target.first, target.second, onCancel = { newTabIn = null }) { pane ->
                newTabIn = null; onDone(); nav.openDetail(target.first, pane)
            }
            return@Column
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            SectionTitle(stringResource(R.string.detail_switch_to), Modifier.weight(1f))
            Segmented(listOf(stringResource(R.string.common_view_list), stringResource(R.string.common_view_workspace)), if (grouped) 1 else 0, { core.groupByWorkspace.set(it == 1) }, compact = true)
        }
        if (grouped) {
            state.workspaces.forEach { ws ->
                Block {
                    Row(Modifier.padding(start = 14.dp, top = 10.dp, end = 14.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, 16.dp)
                        Text(ws.label, style = McType.bodyStrong, color = c.text)
                        Text(ws.serverName, style = McType.path, color = c.muted)
                    }
                    ws.tabs.forEach { tab ->
                        val isCurrent = tab.tabId == current.tabId && ws.serverId == current.serverId
                        val agent = state.agents.firstOrNull { it.serverId == ws.serverId && it.tabId == tab.tabId }
                        val line = agent?.let { TabLine.Agent(it) } ?: tab.firstPaneId?.let { TabLine.Shell(ws, tab, it) }
                        Row(Modifier.fillMaxWidth().background(if (isCurrent) c.inset else Color.Transparent), verticalAlignment = Alignment.CenterVertically) {
                            Row(
                                Modifier.weight(1f).height(48.dp)
                                    .combinedClickable(onClick = { tab.firstPaneId?.let { onDone(); nav.openDetail(ws.serverId, it) } }, onLongClick = line?.let { l -> { onOptions(l) } }, onLongClickLabel = options).padding(start = 28.dp, end = 4.dp),
                                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                Text(tab.label, style = McType.body, color = c.text)
                                tab.agentType?.let { Text(it, style = McType.path, color = c.muted) }
                                Box(Modifier.weight(1f))
                                Text(tabStatus(tab.status), style = McType.caption, color = if (tab.status == Status.BLOCKED) c.accentText else c.muted)
                            }
                            IconAction(McIcons.Close, stringResource(R.string.detail_close_tab, tab.label), iconSize = 14.dp) { confirm = Triple(ws.serverId, tab.tabId, "${ws.label} · ${tab.label}") }
                        }
                    }
                    Row(
                        Modifier.fillMaxWidth().height(44.dp).clickable { newTabIn = ws.serverId to ws.workspaceId }.padding(start = 28.dp),
                        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) { McIcon(McIcons.Plus, c.muted, 14.dp); Text(stringResource(R.string.detail_new_tab), style = McType.small, color = c.muted) }
                }
            }
        } else {
            Block {
                // Every tab, also those without an agent (a workspace with only a shell).
                tabLines(state).forEach { line ->
                    when (line) {
                        is TabLine.Agent -> {
                            val a = line.a
                            val isCurrent = a.paneId == current.paneId && a.serverId == current.serverId
                            Row(Modifier.fillMaxWidth().background(if (isCurrent) c.inset else Color.Transparent), verticalAlignment = Alignment.CenterVertically) {
                                Row(
                                    Modifier.weight(1f).height(52.dp)
                                        .combinedClickable(onClick = { onDone(); nav.openDetail(a.serverId, a.paneId) }, onLongClick = { onOptions(line) }, onLongClickLabel = options).padding(start = 14.dp, end = 4.dp),
                                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
                                ) {
                                    WorkspaceMark(core, a.serverId, a.workspaceId, a.workspace ?: "", tab = a.tab)
                                    Column(Modifier.weight(1f)) {
                                        EllipsisText(agentTitleOf(core, a), McType.body, c.text)
                                        EllipsisText(pathLineOf(core, a), McType.path, c.muted)
                                    }
                                    Text(statusText(a), style = McType.caption, color = if (a.status == Status.BLOCKED) c.accentText else c.muted)
                                }
                                IconAction(McIcons.Close, stringResource(R.string.detail_close_tab, a.tab.orEmpty()), iconSize = 14.dp) { confirm = Triple(a.serverId, a.tabId, "${a.workspace} · ${a.tab}") }
                            }
                        }
                        is TabLine.Shell -> {
                            val (ws, tab) = line.ws to line.tab
                            val isCurrent = line.paneId == current.paneId && ws.serverId == current.serverId
                            Row(Modifier.fillMaxWidth().background(if (isCurrent) c.inset else Color.Transparent), verticalAlignment = Alignment.CenterVertically) {
                                Row(
                                    Modifier.weight(1f).height(52.dp)
                                        .combinedClickable(onClick = { onDone(); nav.openDetail(ws.serverId, line.paneId) }, onLongClick = { onOptions(line) }, onLongClickLabel = options).padding(start = 14.dp, end = 4.dp),
                                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
                                ) {
                                    WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, tab = tab.label)
                                    Column(Modifier.weight(1f)) {
                                        EllipsisText(tabTitleOf(core, tab.label, ws.label, tab.label), McType.body, c.text)
                                        EllipsisText(listOfNotNull(ws.label, tab.cwd?.substringAfterLast('/')).joinToString(" › "), McType.path, c.muted)
                                    }
                                    Text(tab.agentType ?: stringResource(R.string.mission_shell), style = McType.caption, color = c.muted)
                                }
                                IconAction(McIcons.Close, stringResource(R.string.detail_close_tab, tab.label), iconSize = 14.dp) { confirm = Triple(ws.serverId, tab.tabId, "${ws.label} · ${tab.label}") }
                            }
                        }
                    }
                }
                Row(
                    Modifier.fillMaxWidth().height(46.dp).clickable { newTabIn = current.serverId to current.workspaceId }.padding(start = 14.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) { McIcon(McIcons.Plus, c.muted, 14.dp); Text(stringResource(R.string.detail_new_tab_in, current.workspace), style = McType.small, color = c.muted) }
            }
        }
    }
    confirm?.let { (server, tab, label) -> CloseTabDialog(core, server, tab, label) { confirm = null } }
}
