package ch.inscribe.missionctrl

import android.content.Context
import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.Connection
import ch.inscribe.missionctrl.core.FleetState
import ch.inscribe.missionctrl.core.PaneText
import ch.inscribe.missionctrl.core.ServerRow
import ch.inscribe.missionctrl.core.Status
import ch.inscribe.missionctrl.core.TabRow
import ch.inscribe.missionctrl.core.WorkspaceRow
import java.util.Locale
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Sample agents without a server: a first look at the app, Google's review and the store
 * screenshots. One waiting with a menu question (answering it lets it work, then finish), one
 * working, one done, one ready, and a shell. Text to any of the others gets a set reply, so typing
 * never goes nowhere; nothing here thinks. Screens look like the agents' own; their prose is in
 * the app's language. [onChange] runs after every change.
 */
class DemoFleet(private val context: Context, private val scope: CoroutineScope, private val onChange: () -> Unit) {
    companion object {
        const val SERVER = "demo"
        private const val PROMPT = "dev@webshop:~/webshop$ "
    }

    private class Pane(
        val id: String, val workspaceId: String, val workspace: String, val tabId: String, val tab: String,
        val agentType: String?, var status: Status, var screen: String, val cwd: String,
    )

    private fun s(id: Int, vararg args: Any) = context.getString(id, *args)

    /** The language the sample texts were made in: another one makes [Core] build the demo anew. */
    val locale: Locale = context.resources.configuration.locales[0]

    /** Stops the agents that are still "working" (the demo is built anew). */
    fun stop() {
        jobs.values.forEach { it.cancel() }
        jobs.clear()
    }

    private val serverName = s(R.string.demo_server)
    private var cursor = 0
    private val jobs = mutableMapOf<String, Job>()
    private var revision = 1L

    private val panes = listOf(
        Pane("d1", "w1", "Webshop", "t1", "API", "claude", Status.BLOCKED, "", "/home/dev/webshop"),
        Pane("d2", "w1", "Webshop", "t2", "Frontend", "codex", Status.WORKING, "", "/home/dev/webshop/web"),
        Pane("d3", "w1", "Webshop", "t3", "Shell", null, Status.UNKNOWN, "", "/home/dev/webshop"),
        Pane("d4", "w2", "Docs", "t4", "Changelog", "claude", Status.DONE, "", "/home/dev/docs"),
        Pane("d5", "w2", "Docs", "t5", s(R.string.demo_tab_translate), "claude", Status.IDLE, "", "/home/dev/docs"),
    ).also { list ->
        list[1].screen = working(s(R.string.demo_frontend_task), "Working (2m 14s • esc to interrupt)", codex = true)
        list[2].screen = "${PROMPT}git status\nOn branch main\nnothing to commit, working tree clean\n$PROMPT"
        list[3].screen = answer(s(R.string.demo_changelog_answer))
        list[4].screen = answer(s(R.string.demo_translate_answer))
    }

    init { panes[0].screen = question() }

    private val options get() = listOf(s(R.string.demo_option_stripe), s(R.string.demo_option_paypal), s(R.string.demo_option_both), "Type something.")

    /** Claude asking with a menu, cursor on [cursor]. */
    private fun question(): String {
        val rows = options.mapIndexed { i, o ->
            val mark = if (i == cursor) "❯" else " "
            " $mark ${i + 1}. $o" + if (i == 0) "\n      " + s(R.string.demo_option_stripe_hint) else ""
        }
        return "⏺ " + s(R.string.demo_api_intro) + "\n\n" + s(R.string.demo_api_question) + "\n\n" +
            rows.joinToString("\n") + "\n\nEnter to select · ↑/↓ to navigate · Esc to cancel"
    }

    private fun working(task: String, line: String, codex: Boolean = false, steps: Boolean = codex) =
        if (codex) "› $task\n\n" + (if (steps) "• Explored\n  └ Read src/Cart.tsx\n\n" else "") + "• $line\n\n› "
        else "> $task\n\n✻ $line\n\n> "

    private fun answer(text: String, codex: Boolean = false) = if (codex) "• $text\n\n› " else "⏺ $text\n\n> "

    fun state(): FleetState {
        val agents = panes.filter { it.agentType != null }.map { p ->
            AgentRow(SERVER, serverName, p.id, p.workspaceId, p.tabId, p.agentType!!, p.agentType, null, p.status, "", null, p.workspace, p.tab, p.cwd, false)
        }
        val workspaces = panes.groupBy { it.workspaceId }.map { (wsId, list) ->
            val status = when {
                list.any { it.status == Status.BLOCKED } -> Status.BLOCKED
                list.any { it.status == Status.WORKING } -> Status.WORKING
                list.any { it.status == Status.DONE } -> Status.DONE
                else -> Status.IDLE
            }
            WorkspaceRow(SERVER, serverName, wsId, list.first().workspace, status,
                list.map { TabRow(it.tabId, it.tab, it.status, 1u, it.id, it.cwd, it.agentType) }, false, list.first().cwd)
        }
        val server = ServerRow(SERVER, serverName, Connection.Online, "0.9.5", false, workspaces.size.toUInt(), panes.size.toUInt())
        return FleetState(listOf(server), workspaces, agents)
    }

    fun screen(paneId: String): PaneText =
        PaneText(panes.firstOrNull { it.id == paneId }?.screen.orEmpty(), revision.toULong(), false)

    /**
     * Keys or text into a pane: the waiting one takes an answer (arrows, digits, Enter, text), the
     * others answer text with a set reply.
     */
    fun input(paneId: String, keys: List<String> = emptyList(), text: String? = null) {
        val p = panes.firstOrNull { it.id == paneId } ?: return
        val typed = text?.takeIf { it.isNotBlank() }
        if (p.status != Status.BLOCKED) { typed?.let { reply(p, it) }; return }
        var picked: String? = typed
        for (k in keys) {
            when {
                k == "down" -> cursor = (cursor + 1).coerceAtMost(options.lastIndex)
                k == "up" -> cursor = (cursor - 1).coerceAtLeast(0)
                k == "enter" && picked == null && cursor < options.lastIndex -> picked = options[cursor]
                k.length == 1 && k[0] in '1'..'3' -> picked = options[k[0] - '1']
            }
        }
        val chosen = picked ?: run { p.screen = question(); changed(); return }
        later(p, working(chosen, s(R.string.demo_api_working)), 4_000) { answer(s(R.string.demo_api_done, chosen)) }
    }

    /** The set reply: an agent works a moment, then says this is the demo; the shell says so at once. */
    private fun reply(p: Pane, text: String) {
        if (p.agentType == null) {
            p.screen = (p.screen + text + "\n" + s(R.string.demo_shell_reply) + "\n" + PROMPT).lines().takeLast(30).joinToString("\n")
            changed()
            return
        }
        val codex = p.agentType == "codex"
        val asked = if (codex) "› $text\n\n" else "> $text\n\n"
        later(p, working(text, s(R.string.demo_reply_working), codex, steps = false), 3_000) { asked + answer(s(R.string.demo_reply), codex) }
    }

    /** [p] works with [now] on its screen, after [ms] it is done with [done]. */
    private fun later(p: Pane, now: String, ms: Long, done: () -> String) {
        p.status = Status.WORKING
        p.screen = now
        changed()
        jobs.remove(p.id)?.cancel()
        jobs[p.id] = scope.launch {
            delay(ms)
            p.status = Status.DONE
            p.screen = done()
            changed()
        }
    }

    private fun changed() { revision++; onChange() }
}
