package ch.inscribe.missionctrl.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.app.NotificationManagerCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.FleetService
import ch.inscribe.missionctrl.Pin
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.FleetState
import ch.inscribe.missionctrl.core.Status
import ch.inscribe.missionctrl.core.TabRow
import ch.inscribe.missionctrl.core.WorkspaceRow
import kotlinx.coroutines.delay

/** Where the user is. */
sealed interface Screen {
    data object Mission : Screen
    data object Workspaces : Screen
    data class Workspace(val serverId: String, val workspaceId: String) : Screen
    data class Detail(val serverId: String, val paneId: String) : Screen
    data object Settings : Screen
    data object Servers : Screen
    data class ServerEdit(val serverId: String?) : Screen
    data object Keys : Screen
    data class Bar(val which: String) : Screen
    data object Speech : Screen
    data object Language : Screen
    data object Licenses : Screen
    data object Terminal : Screen
    data object Background : Screen
    data object Agents : Screen
}

/** A pane the detail view can show, with what's needed to render its header. */
data class PaneRef(
    val serverId: String,
    val serverName: String,
    val paneId: String,
    val tabId: String,
    val workspaceId: String,
    val workspace: String,
    val tab: String,
    val agentType: String?,
    val customName: String? = null,
)

class Nav(start: Screen) {
    val stack = mutableStateListOf(start)
    val current: Screen get() = stack.last()
    fun go(screen: Screen) { if (current != screen) stack.add(screen) }
    /** Switch a top-level destination (dock). */
    fun root(screen: Screen) { stack.clear(); stack.add(screen) }
    fun back(): Boolean = if (stack.size > 1) { stack.removeAt(stack.lastIndex); true } else false
    var lastDetail: Screen.Detail? by mutableStateOf(null)
    /** The menu sheet (☰), used when the dock is off. */
    var menuOpen by mutableStateOf(false)
    /** The sheet that pins an open tab to the dock (+ in the dock and the menu). */
    var pinPickerOpen by mutableStateOf(false)

    /** The terminal last open, else the first agent's (dock and menu). */
    fun openTerminal(state: FleetState) {
        val d = lastDetail ?: state.agents.firstOrNull()?.let { Screen.Detail(it.serverId, it.paneId) } ?: return
        root(Screen.Mission); go(d)
    }
}

fun AgentRow.ref() = PaneRef(serverId, serverName, paneId, tabId, workspaceId, workspace ?: "?", tab ?: "?", agentType, customName)

/**
 * Agents in herdr's order: servers, then workspaces and tabs exactly as herdr shows them. The
 * list and the grouped view differ only by the headers;
 * what waits shows up top as cards anyway. [grouped] is kept for the callers' sake.
 */
@Suppress("UNUSED_PARAMETER")
fun orderedAgents(state: FleetState, grouped: Boolean): List<AgentRow> {
    val order = HashMap<String, Int>()
    for (ws in state.workspaces) for (tab in ws.tabs) order.putIfAbsent("${ws.serverId}|${tab.tabId}", order.size)
    // Stable sort: several panes of one tab keep herdr's pane order.
    return state.agents.sortedBy { order["${it.serverId}|${it.tabId}"] ?: Int.MAX_VALUE }
}

/** One line of the start page and the switcher: an agent, or a tab without one (a shell). */
sealed interface TabLine {
    val serverId: String
    val workspaceId: String
    val workspace: String
    val serverName: String
    val paneId: String
    val tabId: String
    val tabLabel: String

    data class Agent(val a: AgentRow) : TabLine {
        override val serverId get() = a.serverId
        override val workspaceId get() = a.workspaceId
        override val workspace get() = a.workspace ?: "?"
        override val serverName get() = a.serverName
        override val paneId get() = a.paneId
        override val tabId get() = a.tabId
        override val tabLabel get() = a.tab.orEmpty()
    }

    data class Shell(val ws: WorkspaceRow, val tab: TabRow, override val paneId: String) : TabLine {
        override val serverId get() = ws.serverId
        override val workspaceId get() = ws.workspaceId
        override val workspace get() = ws.label
        override val serverName get() = ws.serverName
        override val tabId get() = tab.tabId
        override val tabLabel get() = tab.label
    }

    fun ref(): PaneRef = when (this) {
        is Agent -> a.ref()
        is Shell -> PaneRef(ws.serverId, ws.serverName, paneId, tab.tabId, ws.workspaceId, ws.label, tab.label, tab.agentType)
    }

    val key: String get() = when (this) {
        is Agent -> "a-" + a.serverId + a.paneId
        is Shell -> "t-" + ws.serverId + tab.tabId
    }
}

/**
 * Every tab in herdr's order: its agents, or the tab itself when no agent runs in it, so a
 * workspace with only a shell shows up too.
 */
fun tabLines(state: FleetState): List<TabLine> {
    val agents = orderedAgents(state, false)
    val byTab = agents.groupBy { it.serverId to it.tabId }
    val out = ArrayList<TabLine>()
    val seen = HashSet<Pair<String, String>>()
    for (ws in state.workspaces) for (tab in ws.tabs) {
        val key = ws.serverId to tab.tabId
        val list = byTab[key]
        if (list != null) { if (seen.add(key)) list.forEach { out += TabLine.Agent(it) } }
        else tab.firstPaneId?.let { out += TabLine.Shell(ws, tab, it) }
    }
    agents.filter { (it.serverId to it.tabId) !in seen }.forEach { out += TabLine.Agent(it) }
    return out
}

/** The pane a tap on a workspace opens: a waiting tab first, else the first tab. */
fun WorkspaceRow.mainPaneId(): String? =
    (tabs.firstOrNull { it.status == Status.BLOCKED } ?: tabs.firstOrNull())?.firstPaneId

/** Pages for the detail pager: every tab in list order (agents and shells), plus the target if it is another pane. */
@Suppress("UNUSED_PARAMETER")
fun detailPages(state: FleetState, grouped: Boolean, target: Screen.Detail): List<PaneRef> {
    val agents = tabLines(state).map { it.ref() }
    if (agents.any { it.serverId == target.serverId && it.paneId == target.paneId }) return agents
    for (ws in state.workspaces.filter { it.serverId == target.serverId }) {
        for (tab in ws.tabs) {
            if (tab.firstPaneId == target.paneId) {
                return agents + PaneRef(ws.serverId, ws.serverName, target.paneId, tab.tabId, ws.workspaceId, ws.label, tab.label, tab.agentType)
            }
        }
    }
    return agents
}

@Composable
fun McApp(core: Core) {
    val mode by core.themeMode.collectAsState()
    val accent by core.accent.collectAsState()
    McTheme(mode, accent) {
        val c = LocalMc.current
        val nav = core.nav
        BackHandler(enabled = nav.stack.size > 1) { nav.back() }
        var askBackground by remember { mutableStateOf(!core.backgroundAsked) }
        if (askBackground) BackgroundQuestion(core) { askBackground = false }
        UnlockOnStart(core)
        val open by core.openRequest.collectAsState()
        LaunchedEffect(open) { open?.let { (server, pane) -> nav.openDetail(server, pane); core.openRequest.value = null } }
        val wide = LocalWindowInfo.current.containerDpSize.width >= 600.dp
        val compactKeys by core.compactKeys.collectAsState()
        CompositionLocalProvider(LocalKeys provides if (compactKeys) KeySize.COMPACT else KeySize.NORMAL) {
            val imeFix = rememberImeFix()
            val keyboard = imeFix?.let { it > 0 } ?: imeVisible()
            Box(Modifier.fillMaxSize().background(c.bg).statusBarsPadding().navigationBarsPadding().windowInsetsPadding(imeFix?.let { WindowInsets(bottom = it) } ?: WindowInsets.ime)) {
                if (wide) WideLayout(core, nav, keyboard) else NarrowLayout(core, nav, keyboard)
            }
            if (nav.menuOpen) MenuSheet(core, nav)
            if (nav.pinPickerOpen) PinSheet(core) { nav.pinPickerOpen = false }
        }
    }
}

private val mainScreens = setOf(Screen.Mission::class, Screen.Workspaces::class, Screen.Workspace::class, Screen.Detail::class)

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun imeVisible(): Boolean = WindowInsets.isImeVisible

/**
 * 0 when Compose still reserves room for a keyboard the window has closed, else null.
 * Compose waits for the end of a keyboard animation and ignores other inset reports until then;
 * leaving the app mid-animation (Enter, straight to Home, back once the text is sent) can lose that
 * end, and the old height stayed until the activity was recreated. While the app is in front, the
 * window is asked once Compose's value has rested for a moment (an animation changes it every
 * frame). Only a closed keyboard is corrected: during a gesture-driven animation (predictive back)
 * the window may already report the end while the finger rests.
 */
@Composable
private fun rememberImeFix(): Int? {
    val view = LocalView.current
    val reported = WindowInsets.ime.getBottom(LocalDensity.current)
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var actual by remember { mutableStateOf<Int?>(null) }
    LaunchedEffect(reported, lifecycle) {
        actual = null
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (true) {
                delay(500)
                val bottom = ViewCompat.getRootWindowInsets(view)?.getInsets(WindowInsetsCompat.Type.ime())?.bottom ?: continue
                val fix = bottom.takeIf { it == 0 && reported != 0 }
                if (fix != null && fix != actual) Log.i("mc-ui", "keyboard height stale: $reported px, window $bottom px")
                actual = fix
            }
        }
    }
    return actual
}

@Composable
private fun NarrowLayout(core: Core, nav: Nav, keyboard: Boolean) {
    // The keyboard is open: the dock steps aside so the terminal keeps its rows.
    val showDock = core.showDock.collectAsState().value && !keyboard
    val screen = nav.current
    Column(Modifier.fillMaxSize()) {
        Box(Modifier.weight(1f).fillMaxWidth()) { ScreenContent(core, nav, screen, wide = false) }
        if (showDock && screen::class in mainScreens) Dock(core, nav, withDetail = true)
    }
}

@Composable
private fun WideLayout(core: Core, nav: Nav, keyboard: Boolean) {
    val c = LocalMc.current
    val showDock = core.showDock.collectAsState().value && !keyboard
    val collapsed by core.sideCollapsed.collectAsState()
    val screen = nav.current
    // On the unfolded screen the start page lives on the left; the right side shows the selected
    // detail, the workspaces while none is chosen, or any other screen. Not the first agent: in the
    // TUI that took over a pane nobody picked and gave it phone size on the computer.
    val right: Screen = when (screen) {
        Screen.Mission -> nav.lastDetail ?: Screen.Workspaces
        else -> screen
    }
    Column(Modifier.fillMaxSize()) {
        val savedWidth by core.sideWidth.collectAsState()
        val maxWidth = (LocalWindowInfo.current.containerDpSize.width.value / 2f).coerceAtLeast(SIDE_MIN)
        // While dragging, the width follows the finger; it is saved when the finger lifts.
        var dragWidth by remember { mutableStateOf<Float?>(null) }
        val width = if (collapsed && dragWidth == null) SIDE_RAIL else (dragWidth ?: savedWidth).coerceIn(SIDE_RAIL, maxWidth)
        Row(Modifier.weight(1f).fillMaxWidth()) {
            Box(Modifier.width(width.dp).fillMaxHeight()) {
                if (width < SIDE_MIN) SideRail(core, nav) else MissionScreen(core, nav, compact = true)
            }
            SideHandle(
                onDrag = { dx -> dragWidth = ((dragWidth ?: width) + dx).coerceIn(SIDE_RAIL, maxWidth) },
                onEnd = {
                    val w = dragWidth ?: width
                    dragWidth = null
                    // Below the minimum the list folds into the rail; pulled out again it unfolds.
                    if (w < SIDE_MIN) core.sideCollapsed.set(true) else { core.sideCollapsed.set(false); core.sideWidth.set(w) }
                },
                onReset = { core.sideCollapsed.set(false); core.sideWidth.set(SIDE_DEFAULT) },
            )
            Box(Modifier.weight(1f).fillMaxHeight()) { ScreenContent(core, nav, right, wide = true) }
        }
        if (showDock && screen::class in mainScreens) Dock(core, nav, withDetail = false)
    }
}

/**
 * Asked once on first start. Yes: switch on, then the notification permission and the battery
 * exemption.
 */
@Composable
private fun BackgroundQuestion(core: Core, onDone: () -> Unit) {
    val c = LocalMc.current
    val context = LocalContext.current
    // Only notifications: battery optimization is left to Android's own list (Settings › Background).
    val askNotify = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { onDone() }
    var answered by remember { mutableStateOf(false) }
    if (answered) return
    AlertDialog(
        onDismissRequest = {},
        containerColor = c.surface,
        title = { Text(stringResource(R.string.appnav_background_title), style = McType.title, color = c.text) },
        text = {
            Text(stringResource(R.string.appnav_background_text), style = McType.small, color = c.muted)
        },
        confirmButton = {
            TextButton(onClick = {
                answered = true
                core.backgroundAsked = true
                core.background.set(true)
                FleetService.start(context)
                if (notificationsAllowed(context) || Build.VERSION.SDK_INT < 33) onDone() else askNotify.launch(Manifest.permission.POST_NOTIFICATIONS)
            }) { Text(stringResource(R.string.appnav_background_yes), style = McType.body, color = c.accentText) }
        },
        dismissButton = {
            TextButton(onClick = { answered = true; core.backgroundAsked = true; onDone() }) {
                Text(stringResource(R.string.appnav_background_no), style = McType.body, color = c.text)
            }
        },
    )
}

private const val SIDE_RAIL = 64f
private const val SIDE_MIN = 240f
private const val SIDE_DEFAULT = 300f

/** Divider between side list and content: drag to resize, double-tap for the default width. */
@Composable
private fun SideHandle(onDrag: (Float) -> Unit, onEnd: () -> Unit, onReset: () -> Unit) {
    val c = LocalMc.current
    val density = LocalDensity.current
    // The gesture detectors start once; they call the latest lambdas (fresh width, collapsed state).
    val drag by rememberUpdatedState(onDrag)
    val end by rememberUpdatedState(onEnd)
    val reset by rememberUpdatedState(onReset)
    val description = stringResource(R.string.appnav_side_handle_desc)
    Box(
        Modifier.width(14.dp).fillMaxHeight()
            .pointerInput(Unit) {
                detectHorizontalDragGestures(onDragEnd = { end() }, onDragCancel = { end() }) { change, dx ->
                    change.consume(); drag(with(density) { dx.toDp().value })
                }
            }
            .pointerInput(Unit) { detectTapGestures(onDoubleTap = { reset() }) }
            .semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.width(1.dp).fillMaxHeight().background(c.line))
        Box(Modifier.width(4.dp).height(36.dp).clip(RoundedCornerShape(2.dp)).background(c.muted))
    }
}

@Composable
private fun ScreenContent(core: Core, nav: Nav, screen: Screen, wide: Boolean) {
    // Each destination gets its own state: the same slot showing another server's form must
    // not keep the previous one's fields.
    key(screen) { ScreenFor(core, nav, screen, wide) }
}

@Composable
private fun ScreenFor(core: Core, nav: Nav, screen: Screen, wide: Boolean) {
    when (screen) {
        Screen.Mission -> MissionScreen(core, nav, compact = false)
        Screen.Workspaces -> WorkspacesScreen(core, nav)
        is Screen.Workspace -> WorkspaceScreen(core, nav, screen)
        is Screen.Detail -> {
            SideEffect { nav.lastDetail = screen }
            DetailScreen(core, nav, screen, showBack = !wide)
        }
        Screen.Settings -> SettingsScreen(core, nav)
        Screen.Servers -> ServersScreen(core, nav)
        is Screen.ServerEdit -> ServerEditScreen(core, nav, screen.serverId)
        Screen.Keys -> KeysScreen(core, nav)
        is Screen.Bar -> BarEditorScreen(core, nav, screen.which)
        Screen.Speech -> SpeechScreen(core, nav)
        Screen.Language -> LanguageScreen(nav)
        Screen.Licenses -> LicensesScreen(nav)
        Screen.Terminal -> TerminalScreen(core, nav)
        Screen.Background -> BackgroundScreen(core, nav)
        Screen.Agents -> AgentsScreen(core, nav)
    }
}

/** Opens an agent: on the phone as a new screen, unfolded on the right side. */
fun Nav.openDetail(serverId: String, paneId: String) {
    val d = Screen.Detail(serverId, paneId)
    lastDetail = d
    if (current is Screen.Detail) { stack[stack.lastIndex] = d } else go(d)
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Dock(core: Core, nav: Nav, withDetail: Boolean) {
    val c = LocalMc.current
    val pins by core.pins.collectAsState()
    val state by core.state.collectAsState()
    val screen = nav.current
    val live = livePins(pins, state)
    val pickLabel = stringResource(R.string.appnav_pin_tab)

    // The entries; without pins "+ Pin" joins them as one more.
    @Composable
    fun Entries(item: Modifier) {
        DockItem(stringResource(R.string.appnav_mission), McIcons.Mission, screen == Screen.Mission, item) { nav.root(Screen.Mission) }
        DockItem(stringResource(R.string.appnav_workspaces), McIcons.Grid, screen == Screen.Workspaces || screen is Screen.Workspace, item) { nav.root(Screen.Workspaces) }
        if (withDetail) DockItem(stringResource(R.string.appnav_terminal), McIcons.Terminal, screen is Screen.Detail, item) { nav.openTerminal(state) }
        if (live.isEmpty()) DockItem(stringResource(R.string.appnav_add_pin), McIcons.Plus, false, item) { nav.pinPickerOpen = true }
    }

    @Composable
    fun Separator() = Box(Modifier.padding(horizontal = 6.dp).width(1.dp).height(28.dp).background(c.line))

    // The pinned tabs, then + for another one; scrolls sideways when they don't fit.
    @Composable
    fun Pins(modifier: Modifier) {
        LazyRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically, modifier = modifier) {
            items(live, key = { it.first.serverId + it.first.tabId }) { (pin, ws, tab) ->
                var menu by remember { mutableStateOf(false) }
                val description = stringResource(R.string.appnav_pinned_options_desc, pin.workspaceLabel, pin.tabLabel)
                Box {
                    Row(
                        Modifier.height(48.dp).clip(RoundedCornerShape(14.dp)).background(c.surface)
                            .combinedClickable(onClick = { nav.openPin(ws, tab) }, onLongClick = { menu = true })
                            .semantics { contentDescription = description }
                            .padding(horizontal = if (withDetail) 14.dp else 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, 18.dp, tab = tab.label)
                        AgentGlyph(core, tab.agentType, null, size = 14.dp, tab = TabRef(ws.serverId, ws.workspaceId, ws.label, tab.label))
                        // The tab's name as in the lists (with the workspace only if another tab is called the same).
                        Text(tabTitleOf(core, tab.label, ws.label, tab.label), style = McType.small, color = c.text, maxLines = 1,
                            overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 140.dp))
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
                        Text("${pin.workspaceLabel} · ${pin.tabLabel}", style = McType.bodyStrong, color = c.text, modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp))
                        DropdownMenuItem(text = { Text(stringResource(R.string.appnav_open), style = McType.body) }, leadingIcon = { McIcon(McIcons.Open, c.text, 16.dp) },
                            onClick = { menu = false; nav.openPin(ws, tab) })
                        DropdownMenuItem(text = { Text(stringResource(R.string.appnav_move_left), style = McType.body) }, leadingIcon = { McIcon(McIcons.Left, c.text, 16.dp) },
                            onClick = { menu = false; core.movePin(pin, -1) })
                        DropdownMenuItem(text = { Text(stringResource(R.string.appnav_move_right), style = McType.body) }, leadingIcon = { McIcon(McIcons.Right, c.text, 16.dp) },
                            onClick = { menu = false; core.movePin(pin, 1) })
                        DropdownMenuItem(text = { Text(stringResource(R.string.common_unpin), style = McType.body) }, leadingIcon = { McIcon(McIcons.Unpin, c.text, 16.dp) },
                            onClick = { menu = false; core.togglePin(pin) })
                    }
                }
            }
            item(key = "add") { IconAction(McIcons.Plus, pickLabel, size = 48.dp, iconSize = 18.dp) { nav.pinPickerOpen = true } }
        }
    }

    Column {
        Box(Modifier.fillMaxWidth().height(1.dp).background(c.line))
        if (withDetail) {
            // Phone: without pins the entries share the width; with pins they stay compact on the
            // left and the pins follow.
            Row(
                Modifier.fillMaxWidth().height(68.dp).padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Entries(if (live.isEmpty()) Modifier.weight(1f) else Modifier.width(68.dp))
                if (live.isNotEmpty()) { Separator(); Pins(Modifier.weight(1f)) }
            }
        } else {
            // Wide: the group in the middle with fixed-width entries, like a desktop dock; the gear
            // stays right, 56 dp kept free on both sides so the group stays centred.
            BoxWithConstraints(Modifier.fillMaxWidth().height(68.dp).padding(horizontal = 12.dp)) {
                Row(
                    Modifier.align(Alignment.Center).widthIn(max = (maxWidth - 112.dp).coerceAtLeast(0.dp)),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Entries(Modifier.width(68.dp))
                    if (live.isNotEmpty()) { Separator(); Pins(Modifier.weight(1f, fill = false)) }
                }
                Box(Modifier.align(Alignment.CenterEnd)) { IconAction(McIcons.Settings, stringResource(R.string.common_settings)) { nav.go(Screen.Settings) } }
            }
        }
    }
}

/** Pinned tabs that still exist, with their workspace and tab (dock and menu). */
private fun livePins(pins: List<Pin>, state: FleetState): List<Triple<Pin, WorkspaceRow, TabRow>> = pins.mapNotNull { pin ->
    val ws = state.workspaces.firstOrNull { it.serverId == pin.serverId && it.tabs.any { t -> t.tabId == pin.tabId } }
    val tab = ws?.tabs?.firstOrNull { it.tabId == pin.tabId }
    if (ws != null && tab?.firstPaneId != null) Triple(pin, ws, tab) else null
}

private fun Nav.openPin(ws: WorkspaceRow, tab: TabRow) {
    tab.firstPaneId?.let { root(Screen.Mission); openDetail(ws.serverId, it) }
}

/**
 * The menu when the dock is off: ☰ at the top of the lists and first in the terminal's bar
 * open the dock's destinations, the pinned tabs and the settings in a sheet.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MenuSheet(core: Core, nav: Nav) {
    val c = LocalMc.current
    val pins by core.pins.collectAsState()
    val state by core.state.collectAsState()
    val screen = nav.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    fun go(action: () -> Unit) { nav.menuOpen = false; action() }
    ModalBottomSheet(onDismissRequest = { nav.menuOpen = false }, sheetState = sheet, containerColor = c.bg) {
        Column(Modifier.padding(start = 12.dp, end = 12.dp, bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                MenuTile(stringResource(R.string.appnav_mission), McIcons.Mission, screen == Screen.Mission, Modifier.weight(1f)) { go { nav.root(Screen.Mission) } }
                MenuTile(stringResource(R.string.appnav_workspaces), McIcons.Grid, screen == Screen.Workspaces || screen is Screen.Workspace, Modifier.weight(1f)) { go { nav.root(Screen.Workspaces) } }
                MenuTile(stringResource(R.string.appnav_terminal), McIcons.Terminal, screen is Screen.Detail, Modifier.weight(1f)) { go { nav.openTerminal(state) } }
            }
            val live = livePins(pins, state)
            if (live.isNotEmpty()) SectionTitle(stringResource(R.string.appnav_pinned))
            Block {
                if (live.isNotEmpty()) {
                    live.forEachIndexed { i, (pin, ws, tab) ->
                        if (i > 0) Divider()
                        val description = stringResource(R.string.appnav_pinned_desc, pin.workspaceLabel, pin.tabLabel)
                        Row(
                            Modifier.fillMaxWidth().height(48.dp).clickable { go { nav.openPin(ws, tab) } }
                                .semantics { contentDescription = description }.padding(horizontal = 14.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, 18.dp, tab = tab.label)
                            AgentGlyph(core, tab.agentType, null, size = 14.dp, tab = TabRef(ws.serverId, ws.workspaceId, ws.label, tab.label))
                            Text(tabTitleOf(core, tab.label, ws.label, tab.label), style = McType.body, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    Divider()
                }
                // Same as + in the dock.
                RowItem(stringResource(R.string.appnav_pin_tab), icon = McIcons.Plus, onClick = { go { nav.pinPickerOpen = true } })
            }
            Block { RowItem(stringResource(R.string.common_settings), icon = McIcons.Settings, onClick = { go { nav.go(Screen.Settings) } }) }
        }
    }
}

/**
 * Pins an open tab to the dock: the open tabs by workspace; a tap pins one at the end of the
 * dock and closes the sheet. Pinned ones stay listed, dimmed, so none seems to be missing.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PinSheet(core: Core, onDone: () -> Unit) {
    val c = LocalMc.current
    val pins by core.pins.collectAsState()
    val state by core.state.collectAsState()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val pinnedMark = stringResource(R.string.appnav_pinned_mark)
    ModalBottomSheet(onDismissRequest = onDone, sheetState = sheet, containerColor = c.bg) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 12.dp).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Column(Modifier.padding(horizontal = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(stringResource(R.string.appnav_pin_tab), style = McType.title, color = c.text)
                Text(stringResource(R.string.appnav_pin_tab_hint), style = McType.small, color = c.muted)
            }
            // Only tabs with a pane: a pin opens its first pane (livePins).
            val groups = state.workspaces.map { ws -> ws to ws.tabs.filter { it.firstPaneId != null } }.filter { it.second.isNotEmpty() }
            if (groups.isEmpty()) Text(stringResource(R.string.appnav_pin_none), style = McType.small, color = c.muted, modifier = Modifier.padding(horizontal = 8.dp))
            groups.forEach { (ws, tabs) ->
                Row(Modifier.padding(start = 8.dp, top = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, 16.dp)
                    Text(ws.label, style = McType.bodyStrong, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                    Text(ws.serverName, style = McType.path, color = c.muted, maxLines = 1)
                }
                Block {
                    tabs.forEachIndexed { i, tab ->
                        if (i > 0) Divider()
                        val pinned = pins.any { it.serverId == ws.serverId && it.tabId == tab.tabId }
                        val title = tabTitleOf(core, tab.label, ws.label, tab.label)
                        Row(
                            Modifier.fillMaxWidth().height(52.dp)
                                .then(if (pinned) Modifier else Modifier.clickable { core.togglePin(Pin(ws.serverId, tab.tabId, ws.label, tab.label)); onDone() })
                                .semantics { contentDescription = if (pinned) "$title, $pinnedMark" else title }
                                .padding(horizontal = 14.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            AgentGlyph(core, tab.agentType, null, size = 14.dp, tab = TabRef(ws.serverId, ws.workspaceId, ws.label, tab.label))
                            Text(title, style = McType.body, color = if (pinned) c.muted else c.text, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                            if (pinned) {
                                McIcon(McIcons.Pin, c.muted, 14.dp)
                                Text(pinnedMark, style = McType.caption, color = c.muted)
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun MenuTile(label: String, icon: ImageVector, active: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalMc.current
    val color = if (active) c.accentText else c.text
    Column(
        modifier.height(72.dp).clip(RoundedCornerShape(14.dp)).background(if (active) c.accentSoft else c.surface)
            .clickable(role = Role.Tab, onClick = onClick).semantics { contentDescription = label },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterVertically),
    ) {
        McIcon(icon, color, 22.dp)
        Text(label, style = McType.small, color = color)
    }
}

@Composable
private fun DockItem(label: String, icon: ImageVector, active: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalMc.current
    val color = if (active) c.accentText else c.muted
    Column(
        modifier.height(52.dp).clip(RoundedCornerShape(12.dp))
            .clickable(role = Role.Tab, onClick = onClick).semantics { contentDescription = label },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        McIcon(icon, color, 20.dp)
        Text(label, style = McType.caption.copy(fontSize = 11.sp), color = color)
    }
}

/** Collapsed left side on the unfolded screen: colour bars, waiting ones marked. */
@Composable
private fun SideRail(core: Core, nav: Nav) {
    val c = LocalMc.current
    val state by core.state.collectAsState()
    val grouped by core.groupByWorkspace.collectAsState()
    Column(Modifier.fillMaxSize().padding(top = 14.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        IconAction(McIcons.PanelExpand, stringResource(R.string.appnav_expand_list)) { core.sideCollapsed.set(false) }
        Box(Modifier.width(24.dp).height(1.dp).background(c.line))
        orderedAgents(state, grouped).forEach { a ->
            val selected = nav.lastDetail?.let { it.serverId == a.serverId && it.paneId == a.paneId } == true
            Box(
                Modifier.size(44.dp, 48.dp).clip(RoundedCornerShape(12.dp)).background(if (selected) c.inset else Color.Transparent)
                    .clickable { nav.openDetail(a.serverId, a.paneId) }
                    .semantics { contentDescription = "${a.workspace} ${a.tab} – ${a.statusLabel}" },
                contentAlignment = Alignment.Center,
            ) {
                WorkspaceMark(core, a.serverId, a.workspaceId, a.workspace ?: "", 26.dp, tab = a.tab)
                if (a.status == Status.BLOCKED) {
                    Box(Modifier.align(Alignment.TopEnd).padding(6.dp).size(7.dp).clip(RoundedCornerShape(4.dp)).background(c.accentText))
                }
            }
        }
    }
}

/** Notifications may be posted: on Android 13+ the permission, before that the app's switch. */
fun notificationsAllowed(context: Context): Boolean =
    NotificationManagerCompat.from(context).areNotificationsEnabled()
