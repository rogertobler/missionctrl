package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.FleetState
import ch.inscribe.missionctrl.core.Status
import ch.inscribe.missionctrl.core.TabRow
import ch.inscribe.missionctrl.core.WorkspaceRow
import org.junit.Assert.assertEquals
import org.junit.Test

class OrderTest {
    private fun tab(id: String) = TabRow(tabId = id, label = id, status = Status.IDLE, panes = 1u, firstPaneId = "$id:p", cwd = null, agentType = "claude")
    private fun agent(tab: String, status: Status) = AgentRow(
        serverId = "s", serverName = "s", paneId = "$tab:p", workspaceId = tab.substringBefore(':'), tabId = tab, name = tab,
        agentType = "claude", customName = null, status = status, statusLabel = "", title = null, workspace = null, tab = tab, cwd = null, stale = false,
    )

    @Test fun agentsFollowHerdrsTabOrderNotUrgency() {
        // herdr shows w2 first, and in w1 the moved tab t3 before t1.
        val workspaces = listOf(
            WorkspaceRow(serverId = "s", serverName = "s", workspaceId = "w2", label = "B", status = Status.IDLE, tabs = listOf(tab("w2:t1")), stale = false, cwd = null),
            WorkspaceRow(serverId = "s", serverName = "s", workspaceId = "w1", label = "A", status = Status.IDLE, tabs = listOf(tab("w1:t3"), tab("w1:t1")), stale = false, cwd = null),
        )
        // The core lists the working agent first (urgency).
        val agents = listOf(agent("w1:t1", Status.WORKING), agent("w1:t3", Status.IDLE), agent("w2:t1", Status.IDLE))
        val ordered = orderedAgents(FleetState(emptyList(), workspaces, agents), grouped = false)
        assertEquals(listOf("w2:t1", "w1:t3", "w1:t1"), ordered.map { it.tabId })
    }
}
