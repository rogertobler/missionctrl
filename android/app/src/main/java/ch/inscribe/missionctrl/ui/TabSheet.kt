package ch.inscribe.missionctrl.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.Pin
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.AgentRow
import kotlinx.coroutines.launch

/**
 * Long press on a tab in a list (agent or shell): open, rename the tab, its colour and symbol,
 * pin, the workspace's settings, close the tab – without going through the settings.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TabSheet(core: Core, nav: Nav, a: TabLine, onDismiss: () -> Unit) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val state by core.state.collectAsState()
    val looks by core.looks.collectAsState()
    val pins by core.pins.collectAsState()
    val ws = state.workspaces.firstOrNull { it.serverId == a.serverId && it.workspaceId == a.workspaceId }
    val tabLabel = a.tabLabel
    var name by remember(a.tabId) { mutableStateOf(tabLabel) }
    var looksOpen by remember { mutableStateOf(false) }
    var confirmClose by remember { mutableStateOf(false) }
    val pinned = pins.any { it.serverId == a.serverId && it.tabId == a.tabId }
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = c.bg) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                WorkspaceMark(core, a.serverId, a.workspaceId, a.workspace, 28.dp, tab = tabLabel)
                Column(Modifier.weight(1f)) {
                    val title = when (a) {
                        is TabLine.Agent -> agentTitleOf(core, a.a)
                        is TabLine.Shell -> tabTitleOf(core, tabLabel, a.workspace, tabLabel)
                    }
                    EllipsisText(title, McType.title, c.text)
                    EllipsisText(listOfNotNull(a.serverName, a.workspace).joinToString(" · "), McType.path, c.muted)
                }
            }
            Block {
                RowItem(stringResource(R.string.tabsheet_open), icon = McIcons.Open, onClick = { onDismiss(); nav.openDetail(a.serverId, a.paneId) })
                Divider()
                RowItem(stringResource(if (pinned) R.string.tabsheet_unpin else R.string.tabsheet_pin), icon = if (pinned) McIcons.Unpin else McIcons.Pin, onClick = {
                    core.togglePin(Pin(a.serverId, a.tabId, a.workspace, tabLabel))
                })
                if (ws != null) {
                    Divider()
                    RowItem(stringResource(R.string.tabsheet_ws_settings, ws.label), icon = McIcons.Sliders, onClick = { onDismiss(); nav.go(Screen.Workspace(ws.serverId, ws.workspaceId)) })
                }
            }
            // Order lives in herdr: every list, the terminal pages and the laptop follow.
            fun move(what: suspend () -> Unit) = scope.launch { runCatching { what() }.onFailure { context.toast(it.message) } }
            val tabPos = core.tabPosition(a.serverId, a.tabId)
            val wsPos = core.workspacePosition(a.serverId, a.workspaceId)
            SectionTitle(stringResource(R.string.tabsheet_order))
            Block {
                MoveRow(stringResource(R.string.tabsheet_tab), tabPos, onMove = { d -> move { core.moveTab(a.serverId, a.tabId, d) } })
                if (ws != null) {
                    Divider()
                    MoveRow(stringResource(R.string.tabsheet_workspace_named, ws.label), wsPos, onMove = { d -> move { core.moveWorkspace(a.serverId, a.workspaceId, d) } })
                }
            }
            if (ws != null && tabLabel.isNotEmpty()) {
                SectionTitle(stringResource(R.string.tabsheet_tab))
                Block {
                    Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        LabeledField(stringResource(R.string.tabsheet_name), name, { name = it })
                        if (name.isNotBlank() && name != tabLabel) SecondaryButton(stringResource(R.string.tabsheet_rename), Modifier.fillMaxWidth()) {
                            scope.launch {
                                runCatching { core.renameTab(a.serverId, a.tabId, name.trim()) }
                                    .onSuccess { core.moveTabLook(ws, tabLabel, name.trim()); context.toast(resources.getString(R.string.tabsheet_renamed)) }
                                    .onFailure { context.toast(it.message) }
                            }
                        }
                    }
                    Divider()
                    RowItem(stringResource(R.string.tabsheet_color_icon), onClick = { looksOpen = !looksOpen }) {
                        McIcon(if (looksOpen) McIcons.ChevronDown else McIcons.Chevron, c.muted, 16.dp)
                    }
                    if (looksOpen) Box(Modifier.padding(14.dp)) {
                        LookFields(looks.tab(ws.serverId, ws.workspaceId, ws.label, tabLabel), ColorOwner.TAB, stringResource(R.string.tabsheet_agent_icon),
                            inherited = looks.workspaceColorOrNull(ws.serverId, ws.workspaceId, ws.label)) { core.setTabLook(ws, tabLabel, it) }
                    }
                }
                SecondaryButton(stringResource(R.string.tabsheet_close_tab), Modifier.fillMaxWidth(), color = c.danger) { confirmClose = true }
            }
        }
    }
    if (confirmClose) CloseTabDialog(core, a.serverId, a.tabId, tabLabel) { confirmClose = false; onDismiss() }
}

/** "Tab  ↑ ↓": one place up or down; the ends grey out. */
@Composable
private fun MoveRow(label: String, position: Pair<Int, Int>?, onMove: (Int) -> Unit) {
    val c = LocalMc.current
    val (i, n) = position ?: (0 to 1)
    Row(Modifier.fillMaxWidth().padding(start = 14.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = McType.body, color = c.text, modifier = Modifier.weight(1f))
        Text(stringResource(R.string.tabsheet_position, i + 1, n), style = McType.caption, color = c.muted)
        IconAction(McIcons.Up, stringResource(R.string.tabsheet_move_up, label), if (i > 0) c.text else c.line) { if (i > 0) onMove(-1) }
        IconAction(McIcons.Down, stringResource(R.string.tabsheet_move_down, label), if (i < n - 1) c.text else c.line) { if (i < n - 1) onMove(1) }
    }
}
