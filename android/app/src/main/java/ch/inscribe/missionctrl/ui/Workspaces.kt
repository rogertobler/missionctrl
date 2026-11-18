package ch.inscribe.missionctrl.ui

import androidx.annotation.StringRes
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.SavedServer
import ch.inscribe.missionctrl.core.Connection
import ch.inscribe.missionctrl.core.McException
import ch.inscribe.missionctrl.core.Status
import ch.inscribe.missionctrl.core.TabStart
import ch.inscribe.missionctrl.core.WorkspaceRow
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun WorkspacesScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val state by core.state.collectAsState()
    val saved by core.servers.collectAsState()
    val looks by core.looks.collectAsState()
    var newWsOn by remember { mutableStateOf<String?>(null) }
    var sheetFor by remember { mutableStateOf<TabLine?>(null) }
    val dock by core.showDock.collectAsState()
    val menuRight by core.menuRight.collectAsState()
    val demoOn by core.demo.collectAsState()
    // In the demo the sample server stands in for the saved ones: the fleet then has only it.
    val shown = if (demoOn) state.servers.map { it.id to it.name } else saved.map { it.id to it.name }
    val tags = rememberServerTags(core)
    val fixedLayouts = rememberFixedLayouts(core)
    val labels = shown.associate { (id, name) -> id to serverLabel(name, tags[id]) }

    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        item {
            Row(Modifier.fillMaxWidth().padding(start = 4.dp, top = 12.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                if (!dock && !menuRight) IconAction(McIcons.Menu, stringResource(R.string.workspaces_menu), c.text) { nav.menuOpen = true }
                Text(stringResource(R.string.workspaces_title), style = McType.title, color = c.text, modifier = Modifier.weight(1f))
                IconAction(McIcons.Settings, stringResource(R.string.workspaces_settings)) { nav.go(Screen.Settings) }
                if (!dock && menuRight) IconAction(McIcons.Menu, stringResource(R.string.workspaces_menu), c.text) { nav.menuOpen = true }
            }
        }
        shown.forEach { (serverId, plainName) ->
            val row = state.servers.firstOrNull { it.id == serverId }
            val online = row?.connection is Connection.Online
            val serverName = labels[serverId] ?: plainName
            val onCore = row?.backend == SavedServer.BACKEND_CORE
            item(key = "s-" + serverId) {
                Row(Modifier.fillMaxWidth().padding(top = 14.dp, start = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.size(6.dp).clip(RoundedCornerShape(3.dp)).then(if (online) Modifier.background(c.accentText) else Modifier.border(1.dp, c.muted, RoundedCornerShape(3.dp))))
                    Text(serverName.uppercase(), style = McType.section, color = c.muted, modifier = if (demoOn) Modifier else Modifier.clickable { nav.go(Screen.ServerEdit(serverId)) })
                    Box(Modifier.weight(1f))
                    Text(stringResource(if (row == null) R.string.workspaces_passive else connectionShortRes(row.connection, saved.backendOf(serverId))), style = McType.caption, color = c.muted)
                    // herdr-core: workspaces and tabs belong to the Herdr app's layout; changed only with "Edit layout".
                    if (online && !demoOn && serverId !in fixedLayouts) IconAction(McIcons.Plus, stringResource(R.string.workspaces_new_on_server, serverName), size = 36.dp, iconSize = 16.dp) { newWsOn = serverId }
                }
            }
            val tuiBack = onCore && online && "tui" in row.availableBackends && saved.backendOf(serverId) == SavedServer.BACKEND_AUTO
            if (tuiBack && serverId !in core.stayOnCore) {
                item(key = "t-" + serverId) { TuiBackHint(core, serverId, plainName) }
            }
            if (onCore && online) {
                item(key = "h-" + serverId) {
                    Text(stringResource(R.string.workspaces_core_hint), style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp))
                }
            }
            if (row == null) {
                item(key = "p-" + serverId) {
                    Text(stringResource(R.string.workspaces_tap_to_connect), style = McType.small, color = c.muted, modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).clickable { core.connectNow(serverId) }.padding(12.dp))
                }
            }
            state.workspaces.filter { it.serverId == serverId }.forEach { ws ->
                item(key = "w-" + ws.serverId + ws.workspaceId) {
                    var menu by remember { mutableStateOf(false) }
                    MoveMenu(menu, ws.label, core.workspacePosition(ws.serverId, ws.workspaceId), onDismiss = { menu = false }, onMove = { d ->
                        scope.launch { runCatching { core.moveWorkspace(ws.serverId, ws.workspaceId, d) }.onFailure { context.toastError(it) } }
                    })
                    // The workspace with its tabs: a tap opens a terminal, only ⚙ leads to the settings.
                    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface)) {
                        Row(
                            Modifier.fillMaxWidth().heightIn(min = 52.dp)
                                .combinedClickable(onClick = { ws.mainPaneId()?.let { nav.openDetail(ws.serverId, it) } }, onLongClick = if (ws.serverId in fixedLayouts || ws.isUnsorted()) null else { { menu = true } }, onLongClickLabel = stringResource(R.string.workspaces_move))
                                .padding(start = 12.dp),
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            WorkspaceMark(core, ws.serverId, ws.workspaceId, ws.label, 24.dp)
                            if (ws.isUnsorted()) {
                                Column(Modifier.weight(1f)) {
                                    EllipsisText(stringResource(R.string.workspaces_terminals), McType.bodyStrong, c.text)
                                    EllipsisText(unsortedSubtitle(ws, state.agents.count { it.serverId == ws.serverId && it.workspaceId == ws.workspaceId }), McType.caption, c.muted)
                                }
                            } else EllipsisText(ws.label, McType.bodyStrong, c.text, Modifier.weight(1f))
                            if (ws.status == Status.BLOCKED) Pill(stringResource(R.string.status_waiting), c.accentText, c.accentSoft, dot = true)
                            IconAction(McIcons.Sliders, stringResource(R.string.workspaces_ws_settings_desc, ws.label), iconSize = 16.dp) { nav.go(Screen.Workspace(ws.serverId, ws.workspaceId)) }
                        }
                        ws.tabs.forEach { tab ->
                            val agent = state.agents.firstOrNull { it.serverId == ws.serverId && it.tabId == tab.tabId }
                            val line = agent?.let { TabLine.Agent(it) } ?: tab.firstPaneId?.let { TabLine.Shell(ws, tab, it) }
                            Divider()
                            Row(
                                Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                    .combinedClickable(onClick = { tab.firstPaneId?.let { nav.openDetail(ws.serverId, it) } }, onLongClick = line?.let { l -> { sheetFor = l } }, onLongClickLabel = stringResource(R.string.workspaces_options))
                                    .padding(start = 48.dp, end = 14.dp),
                                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                AgentGlyph(core, tab.agentType, null, size = 14.dp, tab = TabRef(ws.serverId, ws.workspaceId, ws.label, tab.label))
                                // Name and path share the row; the status stays at the right edge.
                                Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                    EllipsisText(tabListName(ws, tab), McType.body, c.text, Modifier.weight(1f, fill = false))
                                    EllipsisText(listOfNotNull(tab.agentType ?: "shell", tab.cwd?.substringAfterLast('/')).joinToString(" · "), McType.path, c.muted, Modifier.weight(1f, fill = false))
                                }
                                Text(tabStatus(tab.status), style = McType.caption, color = if (tab.status == Status.BLOCKED) c.accentText else if (tab.status == Status.WORKING) c.text else c.muted)
                            }
                        }
                    }
                }
            }
        }
        if (shown.isEmpty()) {
            item { Text(stringResource(R.string.workspaces_no_servers), style = McType.small, color = c.muted, modifier = Modifier.padding(4.dp)) }
        }
    }

    sheetFor?.let { line -> TabSheet(core, nav, line) { sheetFor = null } }

    newWsOn?.let { serverId ->
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(onDismissRequest = { newWsOn = null }, sheetState = sheet, containerColor = c.bg) {
            NewWorkspaceForm(core, serverId, onCancel = { newWsOn = null }) { pane -> newWsOn = null; nav.openDetail(serverId, pane) }
        }
    }
}

/** Short connection state, or "SSH key locked" when its protected key waits (not "retrying"). */
@Composable
fun connectionText(core: Core, serverId: String, c: Connection): String {
    val locked by core.lockedKeys.collectAsState()
    val saved by core.servers.collectAsState()
    return if (c !is Connection.Online && serverId.let { core.waitsForUnlock(it) } && locked.isNotEmpty()) stringResource(R.string.connection_key_locked)
    else stringResource(connectionShortRes(c, saved.backendOf(serverId)))
}

/** Short connection state for a list row; show it with stringResource. [backend]: the server's saved type. */
@StringRes
fun connectionShortRes(c: Connection, backend: String?): Int = when (c) {
    is Connection.Online -> R.string.workspaces_conn_online
    is Connection.Connecting -> R.string.workspaces_conn_connecting
    is Connection.Reconnecting -> if (!c.absent) R.string.workspaces_conn_reconnecting else when (backend) {
        SavedServer.BACKEND_TUI -> R.string.conn_tui_not_running
        SavedServer.BACKEND_CORE -> R.string.conn_core_not_found
        else -> R.string.conn_no_herdr
    }
    is Connection.Incompatible -> R.string.conn_incompatible
    is Connection.HostKeyUnknown -> R.string.workspaces_conn_host_key_unknown
    is Connection.HostKeyChanged -> R.string.workspaces_conn_host_key_changed
    is Connection.AuthFailed -> R.string.workspaces_conn_auth_failed
    is Connection.Failed -> R.string.workspaces_conn_failed
    is Connection.Offline -> R.string.workspaces_conn_offline
}

/** What to do about a connection state, where the short form is not enough; null if nothing. */
@StringRes
fun connectionLongRes(c: Connection, backend: String?): Int? = when {
    c is Connection.Incompatible -> R.string.conn_incompatible_text
    c is Connection.Reconnecting && c.absent -> when (backend) {
        SavedServer.BACKEND_TUI -> R.string.conn_tui_not_running_text
        SavedServer.BACKEND_CORE -> R.string.conn_core_not_found_text
        else -> R.string.conn_no_herdr_text
    }
    else -> null
}

fun List<SavedServer>.backendOf(serverId: String): String? = firstOrNull { it.id == serverId }?.backend

/** Servers connected to herdr-core. */
@Composable
fun rememberCoreServers(core: Core): Set<String> {
    val state by core.real.collectAsState()
    return remember(state.servers) { state.servers.filter { it.backend == SavedServer.BACKEND_CORE }.mapTo(HashSet()) { it.id } }
}

/**
 * Servers on herdr-core whose layout the app must leave alone ("Edit layout" is off): creating,
 * renaming, moving and closing workspaces and tabs is hidden there instead of failing.
 */
@Composable
fun rememberFixedLayouts(core: Core): Set<String> {
    val state by core.real.collectAsState()
    val saved by core.servers.collectAsState()
    return remember(state.servers, saved) {
        val editing = saved.filter { it.editLayout }.mapTo(HashSet()) { it.id }
        state.servers.filter { it.backend == SavedServer.BACKEND_CORE && it.id !in editing }.mapTo(HashSet()) { it.id }
    }
}

/** Tabs of the workspace herdr-core terminals without a space are collected in: one per terminal. */
const val UNSORTED_TAB_PREFIX = "hc:unsorted:"

/** A tab that stands for one herdr-core terminal without a space. */
fun isLooseTerminalTab(tabId: String) = tabId.startsWith(UNSORTED_TAB_PREFIX)

/** An "auto" server on herdr-core whose herdr TUI runs again: the app does not switch by itself. */
@Composable
private fun TuiBackHint(core: Core, serverId: String, name: String) {
    val c = LocalMc.current
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface).padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.conn_tui_back_title), style = McType.bodyStrong, color = c.text)
        Text(stringResource(R.string.conn_tui_back_text, name), style = McType.small, color = c.muted)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SecondaryButton(stringResource(R.string.conn_tui_back_stay), Modifier.weight(1f)) { core.stayOnCore += serverId }
            PrimaryButton(stringResource(R.string.conn_tui_back_switch), Modifier.weight(1f)) { core.reconnect(serverId) }
        }
    }
}

/** The workspace herdr-core terminals without a space are collected in. */
fun WorkspaceRow.isUnsorted() = workspaceId == UNSORTED_WORKSPACE

/** Matches the core's id for that workspace. */
const val UNSORTED_WORKSPACE = "hc:unsorted"

@Composable
private fun unsortedSubtitle(ws: WorkspaceRow, agents: Int): String {
    val terminals = ws.tabs.sumOf { it.panes.toInt() }
    return listOfNotNull(
        stringResource(R.string.workspaces_terminals_no_space),
        pluralStringResource(R.plurals.workspaces_terminals_count, terminals, terminals),
        if (agents > 0) pluralStringResource(R.plurals.workspaces_agents_count, agents, agents) else null,
    ).joinToString(" · ")
}

/** Tag per server id ("tui"/"core") where a host is listed twice, see [Backends.tagged]. */
@Composable
fun rememberServerTags(core: Core): Map<String, String?> {
    val saved by core.servers.collectAsState()
    val state by core.real.collectAsState()
    val connected = state.servers.associate { it.id to it.backend }
    return remember(saved, connected) { Backends.tagged(saved) { connected[it] } }
}

@Composable
private fun NewWorkspaceForm(core: Core, serverId: String, onCancel: () -> Unit, onCreated: (String) -> Unit) =
    StartForm(
        stringResource(R.string.workspaces_new_workspace), stringResource(R.string.workspaces_ws_name_hint), "", stringResource(R.string.workspaces_ws_cwd_hint),
        withCommand = false, stringResource(R.string.workspaces_cancel), Modifier.padding(horizontal = 12.dp).padding(bottom = 24.dp), onCancel,
        create = { name, cwd, start -> core.createWorkspace(serverId, name, cwd, start) }, onCreated = onCreated)

/** Form for a new tab: name, what to start, directory. Calls [onCreated] with the new pane. */
@Composable
fun NewTabForm(core: Core, serverId: String, workspaceId: String, onCancel: () -> Unit, onCreated: (String) -> Unit) {
    val state by core.state.collectAsState()
    val ws = state.workspaces.firstOrNull { it.serverId == serverId && it.workspaceId == workspaceId }
    StartForm(
        stringResource(R.string.workspaces_new_tab_in, ws?.label ?: stringResource(R.string.workspaces_workspace)), stringResource(R.string.workspaces_tab_name_hint),
        ws?.cwd.orEmpty(), "~", withCommand = true, stringResource(R.string.workspaces_back), Modifier, onCancel,
        create = { name, cwd, start -> core.createTab(serverId, workspaceId, name, cwd, start) }, onCreated = onCreated)
}

/** Name, what to start (shell, Claude, Codex, optionally a command) and directory; [create] returns the new pane. */
@Composable
private fun StartForm(
    title: String, nameHint: String, cwdInitial: String, cwdHint: String, withCommand: Boolean, cancelLabel: String, modifier: Modifier,
    onCancel: () -> Unit, create: suspend (String?, String?, TabStart) -> String, onCreated: (String) -> Unit,
) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("") }
    var starter by remember { mutableIntStateOf(1) }
    var command by remember { mutableStateOf("") }
    var cwd by remember { mutableStateOf(cwdInitial) }
    var busy by remember { mutableStateOf(false) }
    Column(modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(title, style = McType.title, color = c.text)
        LabeledField(stringResource(R.string.workspaces_name), name, { name = it }, nameHint)
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(stringResource(R.string.workspaces_start_with), style = McType.small, color = c.muted)
            Segmented(listOf(stringResource(R.string.workspaces_start_shell), "Claude", "Codex") + if (withCommand) listOf(stringResource(R.string.workspaces_command)) else emptyList(), starter, { starter = it }, Modifier.fillMaxWidth())
        }
        if (starter == 3) LabeledField(stringResource(R.string.workspaces_command), command, { command = it }, stringResource(R.string.workspaces_command_hint), mono = true)
        LabeledField(stringResource(R.string.workspaces_directory), cwd, { cwd = it }, cwdHint, mono = true, address = true)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SecondaryButton(cancelLabel, Modifier.weight(1f), onClick = onCancel)
            PrimaryButton(stringResource(if (busy) R.string.workspaces_creating else R.string.workspaces_create), Modifier.weight(1f)) {
                if (busy) return@PrimaryButton
                busy = true
                val start = when (starter) {
                    1 -> TabStart.Agent("claude")
                    2 -> TabStart.Agent("codex")
                    3 -> TabStart.Command(command)
                    else -> TabStart.Shell
                }
                scope.launch {
                    runCatching { create(name.ifBlank { null }, cwd.ifBlank { null }, start) }
                        .onSuccess(onCreated).onFailure { context.toast(context.errorText(it), long = true) }
                    busy = false
                }
            }
        }
    }
}

/**
 * "Close tab?" – shared by the workspace page and the quick switcher. In herdr-core's group of
 * terminals without a space (tabs starting with [UNSORTED_TAB_PREFIX]) it closes that one
 * terminal instead, and says so.
 */
@Composable
fun CloseTabDialog(core: Core, serverId: String, tabId: String, label: String, paneId: String? = null, onDone: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val single = isLooseTerminalTab(tabId)
    val ws = core.state.collectAsState().value.workspaces.firstOrNull { w -> w.serverId == serverId && w.tabs.any { it.tabId == tabId } }
    ConfirmDialog(
        if (single) stringResource(R.string.workspaces_close_terminal_title, label) else stringResource(R.string.workspaces_close_tab_title, label),
        if (single) stringResource(R.string.workspaces_close_terminal_text) else closeTabText(ws?.takeIf { it.tabs.size == 1 }?.label),
        stringResource(R.string.workspaces_close),
        onConfirm = {
            onDone()
            scope.launch {
                runCatching { if (single) core.closeTerminal(serverId, paneId ?: tabId.removePrefix(UNSORTED_TAB_PREFIX)) else core.closeTab(serverId, tabId) }
                    .onFailure { context.toastError(it, closingTerminal = single) }
            }
        },
        onDismiss = onDone,
    )
}

/** herdr closes a workspace together with its last tab: then the dialog says so ([lastIn] = its name). */
@Composable
private fun closeTabText(lastIn: String?) =
    if (lastIn != null) stringResource(R.string.workspaces_close_last_tab_text, lastIn) else stringResource(R.string.workspaces_close_tab_text)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WorkspaceScreen(core: Core, nav: Nav, screen: Screen.Workspace) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val state by core.state.collectAsState()
    val looks by core.looks.collectAsState()
    val ws = state.workspaces.firstOrNull { it.serverId == screen.serverId && it.workspaceId == screen.workspaceId }
    val fixed = screen.serverId in rememberFixedLayouts(core) || ws?.isUnsorted() == true
    if (ws == null) {
        Column(Modifier.fillMaxSize().padding(16.dp)) {
            IconAction(McIcons.Back, stringResource(R.string.workspaces_back), c.text) { nav.back() }
            Text(stringResource(R.string.workspaces_gone), style = McType.small, color = c.muted)
        }
        return
    }
    val look = looks.workspace(ws.serverId, ws.workspaceId, ws.label)
    val prefs = looks.prefs(ws.serverId, ws.workspaceId, ws.label)
    val color = looks.workspaceColor(ws.serverId, ws.workspaceId, ws.label)
    var editingName by remember { mutableStateOf(false) }
    var nameDraft by remember(ws.label) { mutableStateOf(ws.label) }
    var newTab by remember { mutableStateOf(false) }
    var tabSettings by remember { mutableStateOf<String?>(null) }
    var ordering by remember { mutableStateOf(false) }
    var confirm by remember { mutableStateOf<Pair<String, String>?>(null) } // kind ("tab"/"ws"), id
    fun toast(m: String?) = context.toast(m ?: resources.getString(R.string.workspaces_error))
    fun toast(t: Throwable) = context.toast(context.errorText(t) ?: resources.getString(R.string.workspaces_error), long = t is McException.Layout)
    val leaveField = rememberLeaveField()
    fun rename() {
        // Close the keyboard before the field goes, then save in the background.
        leaveField()
        editingName = false
        val name = nameDraft
        scope.launch { runCatching { core.renameWorkspace(ws.serverId, ws.workspaceId, name) }.onFailure { toast(it) } }
    }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(McIcons.Back, stringResource(R.string.workspaces_back), c.text) { nav.back() }
            Text(serverLabel(ws.serverName, rememberServerTags(core)[ws.serverId]), style = McType.path, color = c.muted, modifier = Modifier.clickable { nav.go(Screen.ServerEdit(ws.serverId)) })
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            ColorBar(color, 40.dp, 5.dp)
            if (editingName) {
                Box(Modifier.weight(1f).height(44.dp).clip(RoundedCornerShape(10.dp)).border(1.dp, c.line, RoundedCornerShape(10.dp)).background(c.surface).padding(horizontal = 10.dp), contentAlignment = Alignment.CenterStart) {
                    PrivateTextField(nameDraft, { nameDraft = it }, stringResource(R.string.workspaces_name), Modifier.fillMaxWidth(), autoFocus = true, onSubmit = { rename() })
                }
                SecondaryButton(stringResource(R.string.workspaces_done)) { rename() }
            } else {
                Text(ws.label, style = McType.bigTitle, color = c.text, modifier = Modifier.weight(1f, fill = false))
                if (!fixed) IconAction(McIcons.Pencil, stringResource(R.string.workspaces_rename)) { editingName = true }
            }
        }

        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                SectionTitle(stringResource(R.string.workspaces_tabs), Modifier.weight(1f))
                // Order on the server: the desktop shows the same order afterwards.
                if (ws.tabs.size > 1 && !fixed) Text(
                    stringResource(if (ordering) R.string.workspaces_done else R.string.workspaces_reorder), style = McType.small, color = c.accentText,
                    modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { ordering = !ordering }.padding(horizontal = 10.dp, vertical = 8.dp),
                )
            }
            Block {
                ws.tabs.forEachIndexed { i, tab ->
                    // The same name in the row and in the button descriptions.
                    val name = tabListName(ws, tab)
                    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f).clickable { tab.firstPaneId?.let { nav.openDetail(ws.serverId, it) } }.padding(start = 14.dp, top = 8.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text(name, style = McType.body, color = c.text)
                            EllipsisText(listOfNotNull(tab.agentType ?: "shell", tab.cwd?.substringAfterLast('/')).joinToString("/"), McType.path, c.muted)
                        }
                        Text(tabStatus(tab.status), style = McType.caption, color = if (tab.status == Status.BLOCKED) c.accentText else if (tab.status == Status.WORKING) c.text else c.muted)
                        if (ordering) {
                            fun move(delta: Int) = scope.launch { runCatching { core.moveTab(ws.serverId, tab.tabId, delta) }.onFailure { toast(it) } }
                            IconAction(McIcons.Up, stringResource(R.string.workspaces_move_up, name), if (i > 0) c.text else c.line, iconSize = 16.dp) { if (i > 0) move(-1) }
                            IconAction(McIcons.Down, stringResource(R.string.workspaces_move_down, name), if (i < ws.tabs.lastIndex) c.text else c.line, iconSize = 16.dp) { if (i < ws.tabs.lastIndex) move(1) }
                        } else {
                            IconAction(McIcons.Sliders, stringResource(R.string.workspaces_tab_settings_desc, name), iconSize = 16.dp) { tabSettings = tab.tabId }
                            if (!fixed) IconAction(McIcons.Close, stringResource(R.string.workspaces_close_tab_desc, name), iconSize = 14.dp) { confirm = "tab" to tab.tabId }
                        }
                    }
                    Divider()
                }
                if (!fixed) Row(Modifier.fillMaxWidth().height(48.dp).clickable { newTab = true }.padding(horizontal = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    McIcon(McIcons.Plus, c.muted, 16.dp); Text(stringResource(R.string.workspaces_new_tab), style = McType.body, color = c.muted)
                }
            }
        }

        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.workspaces_look))
            Block {
                Box(Modifier.padding(14.dp)) {
                    LookFields(look, ColorOwner.WORKSPACE, stringResource(R.string.workspaces_no_icon), autoColor = WorkspacePalette[LookBook.autoColorIndex(ws.label)]) { core.setWorkspaceLook(ws, it) }
                }
            }
        }

        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.workspaces_behavior))
            Block {
                // One focus target: the whole row toggles, the box only shows the state.
                Row(Modifier.fillMaxWidth().toggleable(prefs.bindToName, role = Role.Checkbox) { core.setWorkspacePrefs(ws, prefs.copy(bindToName = it)) }.padding(horizontal = 6.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(prefs.bindToName, null, colors = CheckboxDefaults.colors(checkedColor = c.accent, checkmarkColor = c.onAccent))
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.workspaces_bind_to_name, ws.label), style = McType.body, color = c.text)
                        Text(
                            stringResource(if (prefs.bindToName) R.string.workspaces_bind_on else R.string.workspaces_bind_off),
                            style = McType.caption, color = c.muted,
                        )
                    }
                }
                Divider()
                RowItem(stringResource(R.string.workspaces_notifications), stringResource(R.string.workspaces_notifications_subtitle)) {
                    McSwitch(prefs.notify, { core.setWorkspacePrefs(ws, prefs.copy(notify = it)) })
                }
            }
        }

        if (!fixed) SecondaryButton(stringResource(R.string.workspaces_close_workspace), Modifier.fillMaxWidth(), color = c.danger) { confirm = "ws" to ws.workspaceId }
    }

    if (newTab) {
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(onDismissRequest = { newTab = false }, sheetState = sheet, containerColor = c.bg) {
            Box(Modifier.padding(horizontal = 12.dp).padding(bottom = 24.dp)) {
                NewTabForm(core, ws.serverId, ws.workspaceId, onCancel = { newTab = false }) { pane -> newTab = false; nav.openDetail(ws.serverId, pane) }
            }
        }
    }
    tabSettings?.let { tabId ->
        val tab = ws.tabs.firstOrNull { it.tabId == tabId }
        if (tab != null) {
            var label by remember(tabId) { mutableStateOf(tab.label) }
            val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
            ModalBottomSheet(onDismissRequest = { tabSettings = null }, sheetState = sheet, containerColor = c.bg) {
                fun save() {
                    leaveField()
                    tabSettings = null
                    val name = label
                    if (name != tab.label) scope.launch {
                        runCatching { core.renameTab(ws.serverId, tabId, name) }.onSuccess { core.moveTabLook(ws, tab.label, name) }.onFailure { toast(it) }
                    }
                }
                Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(stringResource(R.string.workspaces_tab_named, tab.label), style = McType.title, color = c.text)
                    if (!fixed) LabeledField(stringResource(R.string.workspaces_name), label, { label = it }, onSubmit = { save() })
                    // Right under the name, so saving needs no scrolling past the looks.
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        SecondaryButton(stringResource(R.string.workspaces_cancel), Modifier.weight(1f)) { leaveField(); tabSettings = null }
                        PrimaryButton(stringResource(R.string.workspaces_save), Modifier.weight(1f)) { save() }
                    }
                    val looks by core.looks.collectAsState()
                    LookFields(looks.tab(ws.serverId, ws.workspaceId, ws.label, tab.label), ColorOwner.TAB, stringResource(R.string.workspaces_agent_icon), inherited = looks.workspaceColorOrNull(ws.serverId, ws.workspaceId, ws.label)) { core.setTabLook(ws, tab.label, it) }
                }
            }
        }
    }
    confirm?.let { (kind, id) ->
        val isWs = kind == "ws"
        val label = if (isWs) ws.label else ws.tabs.firstOrNull { it.tabId == id }?.let { tabListName(ws, it) } ?: id
        // The last tab takes the workspace with it: then back to the list, as after closing the workspace.
        val wsGoes = isWs || ws.tabs.size == 1
        ConfirmDialog(
            if (isWs) stringResource(R.string.workspaces_close_ws_title, label) else stringResource(R.string.workspaces_close_tab_title, label),
            if (isWs) pluralStringResource(R.plurals.workspaces_close_ws_text, ws.tabs.size, ws.tabs.size)
            else closeTabText(ws.label.takeIf { wsGoes }),
            stringResource(R.string.workspaces_close),
            onConfirm = {
                confirm = null
                scope.launch {
                    runCatching { if (isWs) core.closeWorkspace(ws.serverId, id) else core.closeTab(ws.serverId, id) }
                        .onSuccess { if (wsGoes) nav.back() }.onFailure { toast(it) }
                }
            },
            onDismiss = { confirm = null },
        )
    }
}
