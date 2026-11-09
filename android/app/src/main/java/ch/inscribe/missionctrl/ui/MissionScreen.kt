package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.core.ServerRow
import androidx.compose.material3.TextButton
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.ui.unit.sp
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.animateFloat
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.SpeechPrefs
import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.Connection
import ch.inscribe.missionctrl.core.Status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** When each agent was first seen waiting (for "4 Min"). App-session only. */
private val blockedSince = mutableStateMapOf<String, Long>()

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun MissionScreen(core: Core, nav: Nav, compact: Boolean) {
    val c = LocalMc.current
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val state by core.state.collectAsState()
    val grouped by core.groupByWorkspace.collectAsState()
    val dismissed by core.dismissed.collectAsState()
    val demoOn by core.demo.collectAsState()
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    var sheetFor by remember { mutableStateOf<TabLine?>(null) }
    val dock by core.showDock.collectAsState()
    val menuRight by core.menuRight.collectAsState()
    LaunchedEffect(Unit) { while (true) { delay(30_000); now = System.currentTimeMillis() } }

    val blocked = orderedAgents(state, grouped).filter { it.status == Status.BLOCKED }
    val blockedKeys = blocked.map { "${it.serverId}|${it.paneId}" }.toSet()
    LaunchedEffect(blockedKeys) {
        blockedKeys.forEach { blockedSince.getOrPut(it) { System.currentTimeMillis() } }
        blockedSince.keys.retainAll(blockedKeys)
    }
    // Closed with ✕: no card while it waits. Answered: the card stays for the next question.
    val closed = dismissed.filterValues { it.question == null }.keys
    val waiting = blocked.filterNot { "${it.serverId}|${it.paneId}" in closed }
    // A waiting agent shows as a card above; closed, it is back in the list.
    val others = tabLines(state).filter { it !is TabLine.Agent || it.a.status != Status.BLOCKED || Core.key(it.a) in closed }
    val issues = state.servers.filter {
        it.connection is Connection.HostKeyUnknown || it.connection is Connection.HostKeyChanged ||
            it.connection is Connection.AuthFailed || it.connection is Connection.Failed
    }

    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item {
            Row(Modifier.fillMaxWidth().padding(start = 4.dp, top = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (!dock && !menuRight) IconAction(McIcons.Menu, stringResource(R.string.common_menu), c.text) { nav.menuOpen = true }
                Text(stringResource(R.string.app_name), style = McType.title, color = c.text, modifier = Modifier.weight(1f))
                if (blocked.isNotEmpty()) Pill(pluralStringResource(R.plurals.mission_waiting_count, blocked.size, blocked.size), c.accentText, c.accentSoft, dot = true)
                if (compact) IconAction(McIcons.PanelCollapse, stringResource(R.string.mission_collapse_list)) { core.sideCollapsed.set(true) }
                else IconAction(McIcons.Settings, stringResource(R.string.common_settings)) { nav.go(Screen.Settings) }
                if (!dock && menuRight) IconAction(McIcons.Menu, stringResource(R.string.common_menu), c.text) { nav.menuOpen = true }
            }
        }
        if (demoOn) item { DemoBanner { core.setDemo(false) } }
        if (state.servers.isEmpty()) {
            item { EmptyFleet(onAdd = { nav.go(Screen.ServerEdit(null)) }, onDemo = { core.setDemo(true) }) }
        }
        items(issues, key = { "issue-" + it.id }) { s ->
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.surface).clickable { nav.go(Screen.ServerEdit(s.id)) }.padding(horizontal = 14.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                McIcon(McIcons.Server, c.danger, 16.dp)
                Text(stringResource(R.string.mission_server_issue, s.name, issueText(s.connection)), style = McType.small, color = c.text, modifier = Modifier.weight(1f))
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
        }
        // Only when someone waits; otherwise the section is gone entirely.
        if (waiting.isNotEmpty()) item { SectionTitle(stringResource(R.string.mission_waiting_for_you), Modifier.padding(top = 6.dp)) }
        items(waiting, key = { "w-" + it.serverId + it.paneId }) { a ->
            WaitingCard(core, nav, a, ageText(blockedSince["${a.serverId}|${a.paneId}"], now), scope)
        }
        item {
            Row(Modifier.fillMaxWidth().padding(top = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                SectionTitle(stringResource(R.string.mission_tabs), Modifier.weight(1f))
                Segmented(listOf(stringResource(R.string.common_view_list), stringResource(R.string.common_view_workspace)), if (grouped) 1 else 0, { core.groupByWorkspace.set(it == 1) }, compact = true)
            }
        }
        if (others.isEmpty() && state.servers.isNotEmpty()) {
            // Right after the start the servers are still connecting: say so instead of an empty list.
            val connecting = state.servers.filter { it.connection is Connection.Connecting || it.connection is Connection.Reconnecting }
            if (connecting.isNotEmpty()) item { ConnectingNote(core, connecting) }
            else item { Text(stringResource(R.string.mission_no_more_tabs), style = McType.small, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp)) }
        }
        if (grouped) {
            val groups = others.groupBy { it.serverId to it.workspaceId }
            groups.forEach { (key, list) ->
                val first = list.first()
                item(key = "g-" + key.first + key.second) {
                    var menu by remember { mutableStateOf(false) }
                    MoveMenu(menu, first.workspace, core.workspacePosition(key.first, key.second), onDismiss = { menu = false }, onMove = { d ->
                        scope.launch { runCatching { core.moveWorkspace(key.first, key.second, d) }.onFailure { context.toast(it.message) } }
                    })
                    // A tap opens the workspace's terminal; only ⚙ leads to its settings.
                    val ws = state.workspaces.firstOrNull { it.serverId == key.first && it.workspaceId == key.second }
                    Row(
                        Modifier.fillMaxWidth().padding(top = 6.dp).clip(RoundedCornerShape(10.dp))
                            .combinedClickable(onClick = { ws?.mainPaneId()?.let { nav.openDetail(key.first, it) } }, onLongClick = { menu = true }, onLongClickLabel = stringResource(R.string.mission_move))
                            .padding(start = 8.dp, top = 2.dp, bottom = 2.dp),
                        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        WorkspaceMark(core, first.serverId, first.workspaceId, first.workspace, 16.dp)
                        Text(first.workspace, style = McType.bodyStrong, color = c.text)
                        Text(first.serverName, style = McType.path, color = c.muted, modifier = Modifier.weight(1f))
                        IconAction(McIcons.Sliders, stringResource(R.string.mission_workspace_settings, first.workspace), size = 40.dp, iconSize = 16.dp) { nav.go(Screen.Workspace(key.first, key.second)) }
                    }
                }
                items(list, key = { it.key }) { line -> TabLineRow(core, nav, line, indent = true, now = now) { sheetFor = it } }
            }
        } else {
            items(others, key = { it.key }) { line -> TabLineRow(core, nav, line, indent = false, now = now) { sheetFor = it } }
        }
    }
    sheetFor?.let { line -> TabSheet(core, nav, line) { sheetFor = null } }
}

@Composable
private fun issueText(c: Connection) = when (c) {
    is Connection.HostKeyUnknown -> stringResource(R.string.mission_issue_host_key_unknown)
    is Connection.HostKeyChanged -> stringResource(R.string.mission_issue_host_key_changed)
    is Connection.AuthFailed -> stringResource(R.string.mission_issue_auth_failed)
    is Connection.Failed -> c.message
    else -> ""
}

@Composable
private fun ageText(since: Long?, now: Long): String {
    if (since == null) return ""
    val min = ((now - since) / 60_000).toInt()
    return if (min < 1) stringResource(R.string.age_now) else stringResource(R.string.age_minutes, min)
}

@Composable
private fun EmptyFleet(onAdd: () -> Unit, onDemo: () -> Unit) {
    val c = LocalMc.current
    Block {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.mission_no_server), style = McType.bodyStrong, color = c.text)
            Text(stringResource(R.string.mission_no_server_hint), style = McType.small, color = c.muted)
            PrimaryButton(stringResource(R.string.mission_add_server), Modifier.fillMaxWidth(), onAdd)
            // Sample agents without a server: a first look, and what Google's review sees.
            SecondaryButton(stringResource(R.string.demo_start), Modifier.fillMaxWidth(), onClick = onDemo)
        }
    }
}

/** The demo is on: sample agents, no server; [onEnd] goes back to the servers. */
@Composable
private fun DemoBanner(onEnd: () -> Unit) {
    val c = LocalMc.current
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.accentSoft).padding(start = 14.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(stringResource(R.string.demo_banner), style = McType.small, color = c.accentText, modifier = Modifier.weight(1f))
        TextButton(onClick = onEnd) { Text(stringResource(R.string.demo_end), style = McType.bodyStrong, color = c.accentText) }
    }
}

/** Servers still connecting while there is nothing to list yet: each with its state. */
@Composable
private fun ConnectingNote(core: Core, servers: List<ServerRow>) {
    val c = LocalMc.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        CircularProgressIndicator(Modifier.size(18.dp).padding(top = 2.dp), color = c.accentText, strokeWidth = 2.dp)
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(stringResource(R.string.mission_connecting), style = McType.body, color = c.text)
            servers.forEach { s ->
                val locked = core.waitsForUnlock(s.id)
                Text(
                    s.name + " · " + connectionText(core, s.id, s.connection), style = McType.small, color = if (locked) c.accentText else c.muted,
                    modifier = if (locked) Modifier.clickable { scope.launch { if (Unlock.ask(context)) core.unlockKeys() } } else Modifier,
                )
            }
        }
    }
}

/** A line of the start page: an agent or a tab without one; long press: options. */
@Composable
private fun TabLineRow(core: Core, nav: Nav, line: TabLine, indent: Boolean, now: Long, onOptions: (TabLine) -> Unit) = when (line) {
    is TabLine.Agent -> AgentLine(core, line.a, indent = indent, now = now, onLongClick = { onOptions(line) }) { nav.openDetail(line.a.serverId, line.a.paneId) }
    is TabLine.Shell -> ShellLine(core, line, indent, onLongClick = { onOptions(line) }) { nav.openDetail(line.serverId, line.paneId) }
}

/** A tab without an agent (a shell): name and place, quieter than an agent and without status. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ShellLine(core: Core, line: TabLine.Shell, indent: Boolean, onLongClick: (() -> Unit)? = null, onClick: () -> Unit) {
    val c = LocalMc.current
    val (ws, tab) = line.ws to line.tab
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).clip(RoundedCornerShape(12.dp))
            .combinedClickable(onClick = onClick, onLongClick = onLongClick, onLongClickLabel = stringResource(R.string.mission_options))
            .padding(start = if (indent) 20.dp else 8.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (!indent) WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, tab = tab.label)
        AgentGlyph(core, tab.agentType, null, tab = TabRef(ws.serverId, ws.workspaceId, ws.label, tab.label))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            EllipsisText(if (indent) tab.label else tabTitleOf(core, tab.label, ws.label, tab.label), McType.body, c.text)
            val dir = tab.cwd?.substringAfterLast('/')
            EllipsisText(if (indent) dir.orEmpty() else listOfNotNull(ws.label, dir).joinToString(" › "), McType.path, c.muted)
        }
        Text(tab.agentType ?: stringResource(R.string.mission_shell), style = McType.small, color = c.muted)
    }
}

/**
 * One agent in the list: mark, name and path line, in the middle what it is doing (its spinner
 * line while it works, the start of its last answer at rest, read from the visible screen while
 * the row is shown), on the right its status and since when. Long press: [onLongClick].
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun AgentLine(core: Core, a: AgentRow, indent: Boolean = false, now: Long = System.currentTimeMillis(), onLongClick: (() -> Unit)? = null, onClick: () -> Unit) {
    val c = LocalMc.current
    val screen by remember(a.serverId, a.paneId) { core.screen(a.serverId, a.paneId) }.collectAsState()
    val working = a.status == Status.WORKING
    val activity = remember(screen, working) { screen?.let { PaneTextTools.activity(it, working) } }
    val since by core.statusSince.collectAsState()
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(12.dp))
            .combinedClickable(onClick = onClick, onLongClick = onLongClick, onLongClickLabel = stringResource(R.string.mission_options))
            .padding(start = if (indent) 20.dp else 8.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (!indent) WorkspaceMark(core, a.serverId, a.workspaceId, a.workspace ?: "", tab = a.tab)
        // Grouped under its workspace: a bar only for a tab with a colour of its own.
        else {
            val looks by core.looks.collectAsState()
            looks.tab(a.serverId, a.workspaceId, a.workspace ?: "", a.tab).ownColor()?.let { ColorBar(it, 18.dp) }
        }
        AgentGlyph(core, a)
        // Name narrower, what it does wider and a little larger.
        Column(Modifier.weight(if (activity != null) 0.7f else 1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            EllipsisText(if (indent) (a.tab ?: a.name) else agentTitleOf(core, a), McType.body.copy(fontWeight = FontWeight.Medium), c.text)
            EllipsisText(if (indent) (a.title ?: a.name) else pathLineOf(core, a), McType.path, c.muted)
        }
        if (activity != null) {
            Text(activity, style = McType.small.copy(fontSize = 14.sp), color = if (working) c.text.copy(alpha = 0.8f) else c.muted, maxLines = 2,
                overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1.3f))
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(2.dp)) {
            StatusIcon(a.status, statusText(a))
            since[Core.key(a)]?.let { Text(durationText(it, now), style = McType.caption, color = c.muted) }
        }
    }
}

/** "just now", "13 min", "2 h", "3 d" since [since]. */
@Composable
fun durationText(since: Long, now: Long): String {
    val min = ((now - since) / 60_000).coerceAtLeast(0)
    return when {
        min < 1 -> stringResource(R.string.mission_duration_now)
        min < 60 -> stringResource(R.string.age_minutes, min)
        min < 24 * 60 -> stringResource(R.string.mission_duration_hours, min / 60)
        else -> stringResource(R.string.mission_duration_days, min / (24 * 60))
    }
}

/**
 * Status as a sign instead of a word: ✓ ready, ✓ in the accent colour done (not seen yet),
 * a hammer that strikes while working, ? waiting for you. [label] is what screen readers say;
 * an unknown status shows it as text.
 */
@Composable
fun StatusIcon(status: Status, label: String) {
    val c = LocalMc.current
    val described = Modifier.semantics { contentDescription = label }
    when (status) {
        Status.IDLE -> Box(described) { McIcon(McIcons.Check, c.muted, 18.dp) }
        Status.DONE -> Box(described) { McIcon(McIcons.Check, c.accentText, 18.dp) }
        Status.BLOCKED -> Box(described) { McIcon(McIcons.Question, c.accentText, 18.dp) }
        Status.WORKING -> {
            // Tilted left, lifted slowly, struck down quickly, a short rest on impact.
            val swing by rememberInfiniteTransition(label = "hammer").animateFloat(
                initialValue = -30f, targetValue = -30f,
                animationSpec = infiniteRepeatable(
                    keyframes {
                        durationMillis = 1200
                        -30f at 0
                        -30f at 200
                        -75f at 520 using FastOutLinearInEasing
                        -75f at 640
                        -30f at 1200 using LinearOutSlowInEasing
                    },
                ),
                label = "swing",
            )
            // Swung around the end of the handle, like a hand holding it.
            Box(described.graphicsLayer { rotationZ = swing; transformOrigin = TransformOrigin(0.5f, 0.9f) }) { McIcon(McIcons.Hammer, c.text, 18.dp) }
        }
        Status.UNKNOWN -> Text(label, style = McType.small, color = c.muted)
    }
}

@Composable
fun tabStatus(s: Status): String = when (s) {
    Status.BLOCKED -> stringResource(R.string.status_waiting)
    Status.DONE -> stringResource(R.string.status_done)
    Status.WORKING -> stringResource(R.string.status_working)
    Status.IDLE -> stringResource(R.string.status_idle)
    Status.UNKNOWN -> ""
}

@Composable
fun statusText(a: AgentRow): String = tabStatus(a.status).ifEmpty { a.statusLabel.ifBlank { "—" } }

@Composable
private fun WaitingCard(core: Core, nav: Nav, a: AgentRow, age: String, scope: CoroutineScope) {
    // [scope] is the screen's, not the card's: the card goes as soon as the agent moves on, often
    // while an answer is still going out, and what follows (on to the terminal after "Chat about
    // this") must still happen.
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val bar by core.startBar.collectAsState()
    /** Ctrl/Alt waiting for the next key. */
    var mod by remember { mutableStateOf<String?>(null) }
    var modFocus by remember { mutableIntStateOf(0) }
    // Visible screen only (a `recent` read would make herdr scroll a resting Claude), shared
    // with the menu check; read while the app is on screen.
    val screen by remember(a.serverId, a.paneId) { core.screen(a.serverId, a.paneId) }.collectAsState()
    val question = remember(screen) { PaneTextTools.question(screen.orEmpty()) }
    val shown = remember(question) { PaneTextTools.unwrap(question) }
    val menu = remember(question) { PaneTextTools.menu(question) }
    val progress = remember(screen) { PaneTextTools.progress(screen.orEmpty()) }
    val review = remember(screen) { PaneTextTools.review(screen.orEmpty()) }
    val planPath = remember(screen) { PaneTextTools.planPath(screen.orEmpty()) }
    // Codex writes its plan into the pane instead of a file ("• Proposed Plan" … "Implement this plan?").
    val inlinePlan = remember(screen) { planPath == null && PaneTextTools.asksToImplementPlan(screen.orEmpty()) }
    var planOpen by remember { mutableStateOf(false) }
    // Answered: "sent" until the next question shows; the same one after ANSWERED_MS again (lost answer?).
    val dismissed by core.dismissed.collectAsState()
    val answeredAt = dismissed[Core.key(a)]?.takeIf { it.question != null }
    var expired by remember(answeredAt) { mutableStateOf(false) }
    LaunchedEffect(answeredAt) { if (answeredAt != null) { delay((answeredAt.at + ANSWERED_MS - System.currentTimeMillis()).coerceAtLeast(0)); expired = true } }
    val answeredHere = answeredAt != null && !expired && answeredAt.question == PaneTextTools.signature(question)
    val currentQuestion by rememberUpdatedState(question)
    var mode by rememberSaveable(a.paneId) { mutableStateOf(Entry.BAR) }
    val draftKey = "card|" + Core.key(a)
    val answer = core.drafts[draftKey].orEmpty()
    fun setAnswer(value: String) { if (value.isEmpty()) core.drafts.remove(draftKey) else core.drafts[draftKey] = value }
    val speech by core.speech.collectAsState()
    val suggestions by core.keyboardSuggestions.collectAsState()
    val (voice, startVoice) = rememberVoice(speech) { heard, complete ->
        if (complete && speech.sendsHeard(core.drafts[draftKey].orEmpty())) {
            scope.launch {
                runCatching { core.answerText(a.serverId, a.paneId, heard) }
                    .onSuccess { route -> core.answered(a, currentQuestion); if (route == PaneTextTools.TextRoute.CHAT) nav.openDetail(a.serverId, a.paneId) }
                    .onFailure { context.toast(it.message) }
            }
        } else { setAnswer((core.drafts[draftKey].orEmpty() + " " + heard).trim()); if (mode != Entry.TYPING) mode = Entry.HEARD }
    }
    val kind = PaneTextTools.kind(question)

    fun send(keys: List<String>) = scope.launch {
        // The card goes only once the answer is out; after an error it stays for another try.
        runCatching { core.sendKeys(a.serverId, a.paneId, keys) }
            .onSuccess { if ("enter" in keys) core.answered(a, currentQuestion) }
            .onFailure { context.toast(it.message) }
    }
    val hideKeyboard = rememberHideKeyboard()
    // The card goes away once answered, so the keyboard closes here whatever the setting says.
    fun submit() = scope.launch {
        // Read now, not from the last frame: the keyboard may commit a correction just before Send.
        val text = core.drafts[draftKey].orEmpty()
        if (text.isBlank()) return@launch
        // Emptied right away, so a second Send (double tap) has nothing to send.
        setAnswer("")
        // The field leaves the card before the answer goes out. The card goes once the agent moves
        // on, and a focused text field that leaves together with its card (or, for a chat, with
        // the whole list) makes Compose stop the app: "pending composition has not been applied"
        // Alone, with the card still there, it goes without trouble.
        val was = mode
        mode = Entry.BAR; hideKeyboard()
        repeat(2) { withFrameNanos { } }
        runCatching { core.answerText(a.serverId, a.paneId, text) }
            .onSuccess { route ->
                core.answered(a, currentQuestion)
                // "Chat about this" expects a discussion: on to the terminal.
                if (route == PaneTextTools.TextRoute.CHAT) nav.openDetail(a.serverId, a.paneId)
            }
            .onFailure { setAnswer(text + core.drafts[draftKey].orEmpty()); mode = was; context.toast(it.message) }
    }

    fun pick(picked: MenuOption) {
        // Counted from the screen as it is now, not as the card last saw it.
        scope.launch {
            // "Type something.", "Tell Claude what to change" and "Chat about this" want text: cursor
            // there, open the field; sending then goes the right way (Core.answerText).
            if (PaneTextTools.takesText(menu, picked)) {
                val arrows = runCatching { PaneTextTools.chooseNow(core.readScreen(a.serverId, a.paneId).text, picked)?.dropLast(1) }.getOrNull()
                if (!arrows.isNullOrEmpty()) send(arrows)
                mode = Entry.TYPING
                return@launch
            }
            val keys = runCatching { PaneTextTools.chooseNow(core.readScreen(a.serverId, a.paneId).text, picked) }.getOrNull()
            if (keys == null) context.toast(resources.getString(R.string.mission_choice_changed)) else send(keys)
        }
    }

    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface)) {
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 4.dp, bottom = 14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                WorkspaceMark(core, a.serverId, a.workspaceId, a.workspace ?: "", 18.dp, tab = a.tab)
                AgentGlyph(core, a)
                Text(agentTitleOf(core, a), style = McType.bodyStrong, color = c.text, modifier = Modifier.clickable { nav.openDetail(a.serverId, a.paneId) })
                EllipsisText(pathLineOf(core, a), McType.path, c.muted, Modifier.weight(1f).clickable { nav.go(Screen.Workspace(a.serverId, a.workspaceId)) })
                if (age.isNotEmpty()) Text(age, style = McType.caption, color = c.accentText)
                IconAction(McIcons.Close, stringResource(R.string.mission_close_card), size = 36.dp, iconSize = 16.dp) { core.dismiss(a) }
            }
            progress?.let { (done, total) ->
                Text(if (done < total) stringResource(R.string.mission_question_progress, done + 1, total) else stringResource(R.string.mission_all_answered, total), style = McType.caption, color = c.accentText)
            }
            // Before Submit: Claude's overview of all answers above "Submit answers / Cancel".
            if (!answeredHere) review?.let { ReviewAnswers(it, Modifier.padding(end = 12.dp)) }
            if (answeredHere) Text(stringResource(R.string.mission_answer_sent), style = McType.small, color = c.muted, modifier = Modifier.padding(end = 12.dp))
            else if (menu != null) {
                // Claude's plan question: the plan itself, to scroll through, with the answers under it.
                if (planPath != null || inlinePlan) Row(
                    Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { planOpen = true }.heightIn(min = 40.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    McIcon(McIcons.File, c.accentText, 16.dp)
                    Text(stringResource(R.string.mission_read_plan), style = McType.bodyStrong, color = c.accentText)
                    McIcon(McIcons.Chevron, c.accentText, 14.dp)
                }
                MenuChoices(menu, Modifier.padding(end = 12.dp), onOpen = { nav.openDetail(a.serverId, a.paneId) }, onPick = ::pick)
            }
            else Text(
                shown.ifBlank { a.title ?: "…" }, style = McType.mono, color = c.mono,
                modifier = Modifier.padding(end = 12.dp).clickable { nav.openDetail(a.serverId, a.paneId) },
            )
        }
        if (!answeredHere) when {
            voice.listening -> ListeningEntry(answer, voice)
            // One branch for both, so a tap into heard text keeps the field and opens the keyboard.
            mode.showsField(answer) -> EntryBar(answer, { setAnswer(it) }, mode, { mode = it }, suggestions, onSpeak = { startVoice() }, onSend = { submit() })
            else -> {
                val keys = PaneTextTools.fitting(bar, kind)
                fun press(k: QuickKey) {
                    // A modifier arms (tapped again: off); armed, a plain key goes out with it.
                    k.mod?.let { m -> mod = if (mod == m) null else m; return }
                    val armed = mod
                    // The keyboard button opens the keyboard for the armed key instead.
                    if (armed != null && k.id == "kb") { modFocus++; return }
                    mod = null
                    if (armed != null && k.keys.size == 1) { hideKeyboard(); send(listOf(ModKeys.chord(armed, k.keys[0]))); return }
                    when (k.id) {
                        "mic" -> startVoice()
                        "kb" -> mode = Entry.TYPING
                        "paste" -> {
                            when (val clip = Attachments.clipboard(context)) {
                                is ClipContent.Text -> { setAnswer(clip.text); mode = Entry.TYPING }
                                else -> nav.openDetail(a.serverId, a.paneId)
                            }
                        }
                        else -> {
                            // ↓/↑ onto "Type something." or "Chat about this" opens the field right away.
                            if (PaneTextTools.landsOnText(menu, k.keys.singleOrNull())) mode = Entry.TYPING
                            send(k.keys)
                        }
                    }
                }
                // No ☰ (the page has its own) and no ⋯: the card only answers; all keys are in the terminal.
                val own = keys.map { k -> BarButton(k.id, stringResource(k.description), k.icon, if (k.icon == null) stringResource(k.label) else null) { press(k) } }
                Column(Modifier.fillMaxWidth().background(c.line), verticalArrangement = Arrangement.spacedBy(1.dp)) {
                    mod?.let { m ->
                        ModBar(m, onReopen = { modFocus++ }) { mod = null; hideKeyboard() }
                        ModCapture(modFocus) { key -> mod = null; hideKeyboard(); send(listOf(ModKeys.chord(m, key))) }
                    }
                    if (own.isNotEmpty()) ButtonBar(own)
                }
            }
        }
    }
    if (planOpen && menu != null) {
        if (planPath != null) {
            PlanSheet(planPath, load = { core.readPlan(a.serverId, planPath) }, menu, onPick = { planOpen = false; pick(it) }, onDismiss = { planOpen = false })
        } else if (inlinePlan) {
            // Above the visible screen too: the plan may be longer than it.
            PlanSheet(null, load = { PaneTextTools.inlinePlan(core.readPane(a.serverId, a.paneId, 800).text).orEmpty() }, menu,
                onPick = { planOpen = false; pick(it) }, onDismiss = { planOpen = false })
        }
    }
}

/** Claude's overview before Submit: each question, small, and the answer under it. */
@Composable
private fun ReviewAnswers(answers: List<Pair<String, String>>, modifier: Modifier) {
    val c = LocalMc.current
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        answers.forEach { (question, answer) ->
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(question, style = McType.small, color = c.muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text("→ " + answer.ifBlank { "–" }, style = McType.body, color = c.accentText)
            }
        }
    }
}

/** Claude's plan, read from the server, to scroll through; its answers under it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlanSheet(path: String?, load: suspend () -> String, menu: Menu, onPick: (MenuOption) -> Unit, onDismiss: () -> Unit) {
    val c = LocalMc.current
    var text by remember(path) { mutableStateOf<String?>(null) }
    var failed by remember(path) { mutableStateOf<String?>(null) }
    val notFound = stringResource(R.string.mission_plan_not_found)
    val loadFailed = stringResource(R.string.mission_plan_load_failed)
    LaunchedEffect(path) {
        runCatching { load() }
            .onSuccess { if (it.isBlank()) failed = notFound else text = it }
            .onFailure { failed = it.message ?: loadFailed }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = c.bg) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(stringResource(R.string.mission_plan), style = McType.title, color = c.text)
            path?.let { Text(it, style = McType.path, color = c.muted) }
            Spacer(Modifier.height(6.dp))
            when {
                text != null -> PlanText(text!!)
                failed != null -> Text(failed!!, style = McType.small, color = c.muted)
                else -> Text(stringResource(R.string.mission_plan_loading), style = McType.small, color = c.muted)
            }
            Spacer(Modifier.height(12.dp))
            MenuChoices(menu, Modifier, onOpen = {}, onPick = onPick)
        }
    }
}

/** Markdown, plainly: headings bold, lists with dots, code in mono. */
@Composable
private fun PlanText(markdown: String) {
    val c = LocalMc.current
    val blocks = remember(markdown) {
        var code = false
        markdown.lines().mapNotNull { line ->
            when {
                line.trimStart().startsWith("```") -> { code = !code; null }
                code -> "code" to line
                line.startsWith("# ") -> "h1" to line.removePrefix("# ")
                line.startsWith("#") -> "h2" to line.trimStart('#').trim()
                line.isBlank() -> "gap" to ""
                line.trimStart().startsWith("- ") || line.trimStart().startsWith("* ") ->
                    "text" to line.takeWhile { it == ' ' } + "• " + line.trimStart().drop(2)
                else -> "text" to line
            }
        }
    }
    blocks.forEach { (kind, line) ->
        when (kind) {
            "code" -> Text(line, style = McType.mono, color = c.mono)
            "h1" -> Text(line, style = McType.title, color = c.text, modifier = Modifier.padding(top = 8.dp))
            "h2" -> Text(line, style = McType.bodyStrong, color = c.text, modifier = Modifier.padding(top = 6.dp))
            "gap" -> Spacer(Modifier.height(2.dp))
            else -> Text(line.replace("**", ""), style = McType.body, color = c.text)
        }
    }
}

/** How long an answered card waits for the next question before it shows the same one again. */
private const val ANSWERED_MS = 15_000L

/** A waiting menu: the question, then one row per option; tapping a row picks it. */
@Composable
private fun MenuChoices(menu: Menu, modifier: Modifier, onOpen: () -> Unit, onPick: (MenuOption) -> Unit) {
    val c = LocalMc.current
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (menu.question.isNotBlank()) Text(
            menu.question, style = McType.body, color = c.text, maxLines = 4, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.clickable(onClick = onOpen),
        )
        menu.options.forEach { o ->
            val description = stringResource(R.string.mission_option_desc, o.number, o.title)
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(if (o.selected) c.accentSoft else c.inset)
                    .clickable(role = Role.Button) { onPick(o) }.padding(horizontal = 12.dp, vertical = 9.dp)
                    .semantics { contentDescription = description },
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text("${o.number}", style = McType.bodyStrong, color = if (o.selected) c.accentText else c.muted, modifier = Modifier.width(16.dp))
                Column(Modifier.weight(1f)) {
                    Text(o.title, style = McType.body, color = c.text)
                    if (o.detail.isNotBlank()) Text(o.detail, style = McType.caption, color = c.muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

/**
 * While listening: what is already in the field plus what was recognized so far, read-only
 * above the bar (up to five lines, the newest in view), then the bar with ✕, the state and ✓.
 */
@Composable
fun ListeningEntry(existing: String, voice: VoiceInput) {
    val c = LocalMc.current
    val keys = LocalKeys.current
    val text = listOf(existing, voice.live).filter { it.isNotBlank() }.joinToString(" ")
    Column(Modifier.fillMaxWidth().background(c.line), verticalArrangement = Arrangement.spacedBy(1.dp)) {
        if (text.isNotBlank()) {
            val scroll = rememberScrollState()
            LaunchedEffect(text, scroll.maxValue) { scroll.scrollTo(scroll.maxValue) }
            Box(Modifier.fillMaxWidth().heightIn(min = keys.height).background(c.surface).padding(horizontal = 14.dp, vertical = (keys.height - 24.dp) / 2), contentAlignment = Alignment.CenterStart) {
                Text("$text …", style = McType.body, color = c.text, modifier = Modifier.heightIn(max = 104.dp).verticalScroll(scroll))
            }
        }
        ListeningBar(voice.partial, onCancel = { voice.stop() }, onDone = { voice.finish() })
    }
}

@Composable
fun ListeningBar(partial: String, onCancel: () -> Unit, onDone: () -> Unit) {
    val c = LocalMc.current
    val h = LocalKeys.current.height
    val cancelDescription = stringResource(R.string.mission_cancel_voice)
    val doneDescription = stringResource(R.string.mission_done_speaking)
    val listening = stringResource(R.string.input_listening)
    Row(Modifier.fillMaxWidth().background(c.line), horizontalArrangement = Arrangement.spacedBy(1.dp)) {
        Box(Modifier.width(56.dp).height(h).background(c.surface).clickable(role = Role.Button, onClick = onCancel).semantics { contentDescription = cancelDescription }, contentAlignment = Alignment.Center) {
            McIcon(McIcons.Close, c.text, 16.dp)
        }
        Box(Modifier.weight(1f).height(h).background(c.surface).padding(horizontal = 8.dp), contentAlignment = Alignment.Center) {
            EllipsisText(partial.ifBlank { listening }, McType.small, c.accentText)
        }
        Box(Modifier.width(56.dp).height(h).background(c.surface).clickable(role = Role.Button, onClick = onDone).semantics { contentDescription = doneDescription }, contentAlignment = Alignment.Center) {
            McIcon(McIcons.Check, c.accentText, 18.dp)
        }
    }
}

/** Where an answer is written: the button bar, our text field with the keyboard, or the field without it (after speech). */
enum class Entry { BAR, TYPING, HEARD }

/**
 * Whether the text field is shown. Heard text with nothing in it falls back to the buttons:
 * speech cancelled, sent on its own, or the microphone not allowed.
 */
fun Entry.showsField(text: String) = this == Entry.TYPING || (this == Entry.HEARD && text.isNotBlank())

/** Speech goes out on its own only into an empty field; otherwise it joins what is there. */
fun SpeechPrefs.sendsHeard(existing: String) = autoSend && existing.isBlank()

/**
 * Our own text field for an answer, the same for speech and keyboard: what was heard lands here
 * without opening the keyboard, a tap into the text opens it for corrections, Enter starts a new
 * line, ➤ sends.
 * [onRaw] (TUI only) switches to sending every key straight to the pane.
 */
@Composable
fun EntryBar(
    value: String,
    onChange: (String) -> Unit,
    entry: Entry,
    onEntry: (Entry) -> Unit,
    suggestions: Boolean,
    onSpeak: () -> Unit,
    onSend: () -> Unit,
    onRaw: (() -> Unit)? = null,
) {
    val c = LocalMc.current
    val keys = LocalKeys.current
    val hideKeyboard = rememberHideKeyboard()
    Column(Modifier.fillMaxWidth().background(c.line), verticalArrangement = Arrangement.spacedBy(1.dp)) {
        Box(Modifier.fillMaxWidth().heightIn(min = keys.height).background(c.surface).padding(horizontal = 14.dp, vertical = (keys.height - 24.dp) / 2), contentAlignment = Alignment.CenterStart) {
            // Recreated when the setting changes: the keyboard reads the input type only once.
            key(suggestions) {
                PrivateTextField(
                    value, onChange, stringResource(R.string.mission_answer_hint), Modifier.fillMaxWidth(), autoFocus = entry == Entry.TYPING, prose = true, suggestions = suggestions,
                    onFocus = { if (it) onEntry(Entry.TYPING) }, onSubmit = onSend,
                )
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(1.dp)) {
            EntryButton(McIcons.Close, stringResource(R.string.mission_discard), Modifier.weight(1f), c.text) { onChange(""); onEntry(Entry.BAR); hideKeyboard() }
            EntryButton(McIcons.Mic, stringResource(R.string.mission_speak), Modifier.weight(1f), c.text) { hideKeyboard(); onEntry(Entry.HEARD); onSpeak() }
            if (onRaw != null) EntryButton(McIcons.Terminal, stringResource(R.string.mission_keys_to_pane), Modifier.weight(1f), c.text, onRaw)
            EntryButton(McIcons.Send, stringResource(R.string.mission_send), Modifier.weight(1.6f), c.accentText, onSend)
        }
    }
}

@Composable
private fun EntryButton(icon: ImageVector, description: String, modifier: Modifier, tint: Color, onClick: () -> Unit) {
    val c = LocalMc.current
    val keys = LocalKeys.current
    Box(modifier.height(keys.height).background(c.surface).clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = description }, contentAlignment = Alignment.Center) {
        McIcon(icon, tint, keys.icon)
    }
}
