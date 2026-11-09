package ch.inscribe.missionctrl

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.annotation.StringRes
import androidx.compose.ui.graphics.toArgb
import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.Connection
import ch.inscribe.missionctrl.core.FleetState
import ch.inscribe.missionctrl.core.ServerRow
import ch.inscribe.missionctrl.core.Status
import ch.inscribe.missionctrl.ui.Names
import ch.inscribe.missionctrl.ui.PaneTextTools
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Keeps the SSH connections open while the app is not on screen.
 *
 * Without a push service this is the only way to hear about a waiting agent: Android cuts a
 * background app's network within seconds. The service shows a permanent status notification
 * ("1 wartet · 2 arbeiten") and posts a notification when an agent starts waiting or is done,
 * or when a server stays unreachable. The connections themselves live in [Core]; the service
 * only keeps the process in the foreground.
 */
class FleetService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var watcher: Job? = null
    private val core: Core by lazy { CoreHolder.get(applicationContext) }

    /** Servers seen online during this run, and since when they are not any more. */
    private val wasOnline = mutableSetOf<String>()
    private val downSince = mutableMapOf<String, Long>()
    private val lostShown = mutableSetOf<String>()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun attachBaseContext(newBase: Context) = super.attachBaseContext(AppLanguage.wrap(newBase))

    override fun onCreate() {
        super.onCreate()
        core.serviceRunning = true
        Notifications.channels(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        val status = Notifications.status(this, core.state.value)
        // The "special use" type exists from Android 14; before that the service needs no type.
        if (Build.VERSION.SDK_INT >= 34) startForeground(Notifications.STATUS_ID, status, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(Notifications.STATUS_ID, status)
        if (watcher == null) {
            watcher = scope.launch { watch() }
            scope.launch { watchConnections() }
            scope.launch { watchStatus() }
            scope.launch { watchLockedKeys() }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        core.serviceRunning = false
        scope.cancel()
        super.onDestroy()
    }

    private suspend fun watch() {
        var last: Map<String, Status>? = null
        core.state.collect { state ->
            trackConnections(state.servers)
            val before = last
            last = state.agents.associate { key(it) to it.status }
            // The first state only sets the baseline: no flood of old news when the service starts.
            if (before == null) return@collect
            for (a in state.agents) {
                val was = before[key(a)] ?: continue
                val allowed = !core.visible && core.notifies(a)
                when {
                    a.status == Status.BLOCKED && was != Status.BLOCKED -> if (allowed && core.notifyWaiting.value) scope.launch { waiting(a) }
                    was == Status.WORKING && (a.status == Status.DONE || a.status == Status.IDLE) ->
                        if (allowed && core.notifyDone.value) Notifications.agent(this, core, a, R.string.fleetservice_agent_done, null, Notifications.CHANNEL_DONE)
                    was == Status.BLOCKED && a.status != Status.BLOCKED -> Notifications.cancel(this, a)
                }
            }
        }
    }

    /** The status line only changes with its numbers (Android limits how often it may be posted). */
    private suspend fun watchStatus() {
        core.state.map { Notifications.Counts.of(it) }.distinctUntilChanged().collect {
            Notifications.manager(this).notify(Notifications.STATUS_ID, Notifications.status(this, it))
        }
    }

    @Synchronized
    private fun trackConnections(servers: List<ServerRow>) {
        val now = System.currentTimeMillis()
        val ids = servers.map { it.id }.toSet()
        downSince.keys.retainAll(ids); wasOnline.retainAll(ids)
        for (s in servers) {
            if (s.connection is Connection.Online) {
                wasOnline += s.id
                downSince.remove(s.id)
                if (lostShown.remove(s.id)) Notifications.cancelConnection(this, s.id)
            } else if (s.id in wasOnline) {
                downSince.getOrPut(s.id) { now }
            }
        }
    }

    /** A server that was online and stays unreachable for [LOST_AFTER_MS] gets one notification. */
    private suspend fun watchConnections() {
        while (scope.isActive) {
            delay(15_000)
            val due = synchronized(this) {
                val now = System.currentTimeMillis()
                downSince.filter { (id, since) -> now - since >= LOST_AFTER_MS && id !in lostShown }.keys.toList()
            }
            for (id in due) {
                if (core.visible || !core.notifyConnection.value || !core.serverNotifies(id)) continue
                val server = core.state.value.servers.firstOrNull { it.id == id } ?: continue
                synchronized(this) { lostShown += id }
                Notifications.connectionLost(this, server)
            }
        }
    }

    /** Started without the app (e.g. after Android ended the process): a protected key needs a tap to unlock. */
    private suspend fun watchLockedKeys() {
        core.lockedKeys.collect {
            if (core.lockedInUse() && !core.visible) Notifications.keysLocked(this) else Notifications.cancelKeysLocked(this)
        }
    }

    private suspend fun waiting(a: AgentRow) {
        val question = core.screenText(a.serverId, a.paneId)?.let { PaneTextTools.question(it, maxLines = 8) }
        // It may have been answered while the screen was read.
        if (core.state.value.agents.none { key(it) == key(a) && it.status == Status.BLOCKED }) return
        Notifications.agent(this, core, a, R.string.fleetservice_agent_waiting, question, Notifications.CHANNEL_WAITING)
    }

    companion object {
        const val ACTION_STOP = "ch.inscribe.missionctrl.STOP"
        const val EXTRA_SERVER = "server"
        const val EXTRA_PANE = "pane"
        const val LOST_AFTER_MS = 60_000L

        fun key(a: AgentRow) = Core.key(a)

        fun start(context: Context) = context.startForegroundService(Intent(context, FleetService::class.java))
        fun stop(context: Context) = context.startService(Intent(context, FleetService::class.java).setAction(FleetService.ACTION_STOP))
    }
}

/**
 * Answers from a notification: a menu option's number, or a typed/spoken reply.
 * Sends only if the agent still waits (and, for an option, the same option of the same question is
 * still on screen), so a late tap never types into whatever the agent does by then.
 */
class ReplyReceiver : BroadcastReceiver() {
    override fun onReceive(received: Context, intent: Intent) {
        // Notifications built here speak the app's language (below Android 13 only through wrap).
        val context = AppLanguage.wrap(received)
        val server = intent.getStringExtra(FleetService.EXTRA_SERVER) ?: return
        val pane = intent.getStringExtra(FleetService.EXTRA_PANE) ?: return
        val option = intent.getStringExtra(EXTRA_OPTION)
        val reply = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(KEY_REPLY)?.toString()?.trim()
        if (intent.action == ACTION_RETRY) { CoreHolder.get(context).retry(server); Notifications.cancelConnection(context, server); return }
        val core = CoreHolder.get(context)
        val pending = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val agent = core.state.value.agents.firstOrNull { it.serverId == server && it.paneId == pane }
                val result = runCatching {
                    check(agent != null && agent.status == Status.BLOCKED) { context.getString(R.string.fleetservice_question_closed) }
                    when {
                        option != null -> {
                            // The same option of the same question, not just any menu.
                            val screen = core.readScreen(server, pane).text
                            val title = intent.getStringExtra(EXTRA_TITLE)
                            val open = if (title != null) PaneTextTools.stillOffers(screen, option, title) else PaneTextTools.showsMenu(screen)
                            check(open) { context.getString(R.string.fleetservice_choice_closed) }
                            core.sendKeys(server, pane, listOf(option))
                        }
                        !reply.isNullOrEmpty() -> core.sendText(server, pane, reply, enter = true)
                        else -> error(context.getString(R.string.fleetservice_no_answer))
                    }
                }
                when {
                    result.isSuccess -> { agent?.let { core.dismiss(it) }; Notifications.cancel(context, server, pane) }
                    agent != null -> Notifications.agent(context, core, agent, R.string.fleetservice_agent_not_sent, result.exceptionOrNull()?.message, Notifications.CHANNEL_WAITING, answers = false)
                    // Pane gone: a reply field would otherwise spin forever.
                    else -> Notifications.cancel(context, server, pane)
                }
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_ANSWER = "ch.inscribe.missionctrl.ANSWER"
        const val ACTION_RETRY = "ch.inscribe.missionctrl.RETRY"
        const val EXTRA_OPTION = "option"
        const val EXTRA_TITLE = "title"
        const val KEY_REPLY = "reply"
    }
}

/** Notification channels and builders. Everything stays on the device. */
object Notifications {
    const val STATUS_ID = 1
    /** Quiet channel (lowest importance); "status" was the first, louder one. */
    const val CHANNEL_STATUS = "status_quiet"
    private const val OLD_CHANNEL_STATUS = "status"
    const val CHANNEL_WAITING = "waiting"
    const val CHANNEL_DONE = "done"
    const val CHANNEL_CONNECTION = "connection"

    /** The user switched the status line off in Android's settings. */
    fun statusHidden(context: Context): Boolean =
        manager(context).getNotificationChannel(CHANNEL_STATUS)?.importance == NotificationManager.IMPORTANCE_NONE

    fun manager(context: Context): NotificationManager = context.getSystemService(NotificationManager::class.java)

    fun channels(context: Context) {
        manager(context).deleteNotificationChannel(OLD_CHANNEL_STATUS)
        manager(context).createNotificationChannels(listOf(
            // Android requires a notification while the service runs; this one stays out of the
            // way (no icon in the status bar, folded at the bottom) and can be switched off.
            NotificationChannel(CHANNEL_STATUS, context.getString(R.string.fleetservice_channel_status), NotificationManager.IMPORTANCE_MIN).apply {
                description = context.getString(R.string.fleetservice_channel_status_desc)
                setShowBadge(false)
            },
            NotificationChannel(CHANNEL_WAITING, context.getString(R.string.fleetservice_channel_waiting), NotificationManager.IMPORTANCE_HIGH).apply {
                description = context.getString(R.string.fleetservice_channel_waiting_desc)
            },
            NotificationChannel(CHANNEL_DONE, context.getString(R.string.fleetservice_channel_done), NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = context.getString(R.string.fleetservice_channel_done_desc)
            },
            NotificationChannel(CHANNEL_CONNECTION, context.getString(R.string.fleetservice_channel_connection), NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = context.getString(R.string.fleetservice_channel_connection_desc)
            },
        ))
    }

    private fun openApp(context: Context, a: AgentRow?): PendingIntent {
        val intent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        if (a != null) intent.putExtra(FleetService.EXTRA_SERVER, a.serverId).putExtra(FleetService.EXTRA_PANE, a.paneId).setIdentifier(FleetService.key(a))
        val code = a?.let { FleetService.key(it).hashCode() } ?: 0
        return PendingIntent.getActivity(context, code, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** What the status line shows. */
    data class Counts(val waiting: Int, val working: Int, val online: Int, val servers: Int) {
        companion object {
            fun of(state: FleetState) = Counts(
                state.agents.count { it.status == Status.BLOCKED }, state.agents.count { it.status == Status.WORKING },
                state.servers.count { it.connection is Connection.Online }, state.servers.size,
            )
        }
    }

    fun status(context: Context, state: FleetState): Notification = status(context, Counts.of(state))

    fun status(context: Context, counts: Counts): Notification {
        val (waiting, working, online, total) = counts
        val res = context.resources
        val parts = buildList {
            if (waiting > 0) add(res.getQuantityString(R.plurals.fleetservice_status_waiting, waiting, waiting))
            if (working > 0) add(res.getQuantityString(R.plurals.fleetservice_status_working, working, working))
            if (isEmpty()) add(context.getString(R.string.fleetservice_status_quiet))
        }
        return Notification.Builder(context, CHANNEL_STATUS)
            .setSmallIcon(R.drawable.ic_stat_mission)
            .setContentTitle(parts.joinToString(" · "))
            .setContentText(res.getQuantityString(R.plurals.fleetservice_status_connected, total, online, total))
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setCategory(Notification.CATEGORY_SERVICE)
            // No "Disconnect" action: it only ended the service, not the connections. A tap opens the app,
            // where Settings › Background & notifications switches monitoring off.
            .setContentIntent(openApp(context, null))
            .build()
    }

    /** Answering changes what runs on a server: only with the phone unlocked (Android 12+). */
    @SuppressLint("NewApi")
    private fun Notification.Action.Builder.unlocked(): Notification.Action.Builder = apply { setAuthenticationRequired(true) }

    private fun answerIntent(context: Context, a: AgentRow, code: Int, mutable: Boolean, extra: (Intent) -> Unit): PendingIntent {
        val intent = Intent(context, ReplyReceiver::class.java).setAction(ReplyReceiver.ACTION_ANSWER)
            .putExtra(FleetService.EXTRA_SERVER, a.serverId).putExtra(FleetService.EXTRA_PANE, a.paneId)
        extra(intent)
        // The identifier keeps intents of different panes apart even if request codes collide.
        intent.identifier = "${FleetService.key(a)}|$code"
        // A reply field needs a mutable intent (the system adds the text).
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE
        return PendingIntent.getBroadcast(context, code, intent, flags)
    }

    /**
     * Up to three menu options as buttons, otherwise a reply field. None before Android 12: there
     * an action cannot require unlocking, and answering from the lock screen is too much.
     */
    private fun answers(context: Context, a: AgentRow, question: String?): List<Notification.Action> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return emptyList()
        val q = question.orEmpty()
        if (PaneTextTools.showsMenu(q)) {
            return PaneTextTools.options(q).take(3).mapIndexed { i, (n, label) ->
                val short = "$n · " + if (label.length > 22) label.take(21) + "…" else label
                Notification.Action.Builder(null, short, answerIntent(context, a, i + 1, false) { it.putExtra(ReplyReceiver.EXTRA_OPTION, n).putExtra(ReplyReceiver.EXTRA_TITLE, label) }).unlocked().build()
            }
        }
        val input = RemoteInput.Builder(ReplyReceiver.KEY_REPLY).setLabel(context.getString(R.string.fleetservice_reply_label)).build()
        return listOf(
            Notification.Action.Builder(null, context.getString(R.string.fleetservice_reply), answerIntent(context, a, 0, true) {})
                .addRemoteInput(input).unlocked().build(),
        )
    }

    /** One notification per pane; a newer one replaces the older (waiting → done). [what] takes the agent's name. */
    fun agent(context: Context, core: Core, a: AgentRow, @StringRes what: Int, text: String?, channel: String, answers: Boolean = true) {
        val name = Names.title(a.tab, a.workspace, a.name, Names.duplicateTabs(core.state.value))
        val title = context.getString(what, name)
        // On the lock screen only that something happened, not what the agent wrote.
        val public = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_stat_mission)
            .setContentTitle(context.getString(if (channel == CHANNEL_WAITING) R.string.fleetservice_public_waiting else R.string.fleetservice_public_done))
            .build()
        val n = Notification.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_stat_mission)
            .apply { core.looks.value.paneColorOrNull(a.serverId, a.workspaceId, a.workspace ?: "", a.tab)?.let { setColor(it.toArgb()) } }
            .setContentTitle(title)
            .setContentText(text?.let(PaneTextTools::headline) ?: a.serverName)
            .apply { if (!text.isNullOrBlank()) style = Notification.BigTextStyle().bigText(text) }
            .setSubText(listOfNotNull(a.serverName, a.workspace).joinToString(" · "))
            .setAutoCancel(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(public)
            .setCategory(if (channel == CHANNEL_WAITING) Notification.CATEGORY_REMINDER else Notification.CATEGORY_STATUS)
            .setContentIntent(openApp(context, a))
            .apply { if (answers && channel == CHANNEL_WAITING) answers(context, a, text).forEach { addAction(it) } }
            .build()
        manager(context).notify(FleetService.key(a).hashCode(), n)
    }

    fun cancel(context: Context, a: AgentRow) = cancel(context, a.serverId, a.paneId)
    fun cancel(context: Context, serverId: String, paneId: String) = manager(context).cancel("$serverId|$paneId".hashCode())

    private fun connectionId(serverId: String) = "conn|$serverId".hashCode()

    fun connectionLost(context: Context, server: ServerRow) {
        val retry = PendingIntent.getBroadcast(
            context, connectionId(server.id),
            Intent(context, ReplyReceiver::class.java).setAction(ReplyReceiver.ACTION_RETRY).putExtra(FleetService.EXTRA_SERVER, server.id).putExtra(FleetService.EXTRA_PANE, ""),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val reason = when (val c = server.connection) {
            is Connection.Reconnecting -> c.error
            is Connection.Failed -> c.message
            is Connection.AuthFailed -> context.getString(R.string.fleetservice_auth_rejected)
            else -> context.getString(R.string.fleetservice_unreachable)
        }
        val missing = context.getString(R.string.fleetservice_lost_text)
        val n = Notification.Builder(context, CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_stat_mission)
            .setContentTitle(context.getString(R.string.fleetservice_lost_title, server.name))
            .setContentText("$missing $reason")
            .setStyle(Notification.BigTextStyle().bigText("$missing\n$reason"))
            .setAutoCancel(true)
            .setCategory(Notification.CATEGORY_ERROR)
            .setContentIntent(openApp(context, null))
            .addAction(Notification.Action.Builder(null, context.getString(R.string.fleetservice_reconnect), retry).build())
            .build()
        manager(context).notify(connectionId(server.id), n)
    }

    fun cancelConnection(context: Context, serverId: String) = manager(context).cancel(connectionId(serverId))

    private const val KEYS_LOCKED_ID = 2

    fun keysLocked(context: Context) {
        val n = Notification.Builder(context, CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_stat_mission)
            .setContentTitle(context.getString(R.string.fleetservice_keys_locked_title))
            .setContentText(context.getString(R.string.fleetservice_keys_locked_text))
            .setStyle(Notification.BigTextStyle().bigText(context.getString(R.string.fleetservice_keys_locked_text)))
            .setCategory(Notification.CATEGORY_ERROR)
            .setContentIntent(openApp(context, null))
            .build()
        manager(context).notify(KEYS_LOCKED_ID, n)
    }

    fun cancelKeysLocked(context: Context) = manager(context).cancel(KEYS_LOCKED_ID)

    /** The app came to the screen: what the notifications said is visible there now. */
    fun clearAgents(context: Context) {
        val m = manager(context)
        m.activeNotifications.filter { it.id != STATUS_ID }.forEach { m.cancel(it.id) }
    }
}
