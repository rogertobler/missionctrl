package ch.inscribe.missionctrl.ui

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.speech.SpeechRecognizer
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.TextUnitType
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.AppLanguage
import ch.inscribe.missionctrl.BuildConfig
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.FleetService
import ch.inscribe.missionctrl.Notifications
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.KeyVault
import ch.inscribe.missionctrl.SavedServer
import ch.inscribe.missionctrl.SpeechPrefs
import ch.inscribe.missionctrl.WhisperModelInfo
import ch.inscribe.missionctrl.WhisperStore
import ch.inscribe.missionctrl.core.Connection
import kotlinx.coroutines.launch

@Composable
internal fun Page(title: String, nav: Nav, content: @Composable () -> Unit) {
    val c = LocalMc.current
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(McIcons.Back, stringResource(R.string.settings_back), c.text) { nav.back() }
            Text(title, style = McType.title, color = c.text)
        }
        content()
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SettingsScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val context = LocalContext.current
    val mode by core.themeMode.collectAsState()
    val accent by core.accent.collectAsState()
    val dock by core.showDock.collectAsState()
    val servers by core.servers.collectAsState()
    val keys by core.keyList.collectAsState()
    val startBar by core.startBar.collectAsState()
    val detailBar by core.detailBar.collectAsState()
    Page(stringResource(R.string.settings_title), nav) {
        Block {
            RowItem(stringResource(R.string.settings_servers), icon = McIcons.Server, onClick = { nav.go(Screen.Servers) }, value = "${servers.size}") {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
            Divider()
            RowItem(stringResource(R.string.settings_ssh_keys), icon = McIcons.Key, onClick = { nav.go(Screen.Keys) }, value = "${keys.size}") {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
            Divider()
            val language = remember { AppLanguage.current(context) }
            RowItem(stringResource(R.string.language), icon = McIcons.SymGlobe, onClick = { nav.go(Screen.Language) }, value = if (language.isEmpty()) stringResource(R.string.language_system) else AppLanguage.displayName(language)) {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
            Divider()
            val speech by core.speech.collectAsState()
            RowItem(stringResource(R.string.settings_speech), icon = McIcons.Mic, onClick = { nav.go(Screen.Speech) }, value = when (speech.engine) { "device" -> stringResource(R.string.settings_speech_engine_device); "whisper" -> "Whisper ${ch.inscribe.missionctrl.WhisperStore.info(speech.whisperModel)?.label(context) ?: ""}".trim(); else -> stringResource(R.string.settings_speech_engine_service) }) {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
            Divider()
            val term by core.terminal.collectAsState()
            RowItem(stringResource(R.string.settings_terminal), icon = McIcons.Terminal, onClick = { nav.go(Screen.Terminal) }, value = "${if (term.tui) "TUI" else stringResource(R.string.settings_term_view_normal)} · ${term.scheme(c.dark).displayName()} · ${TerminalPrefs.sizeLabel(term.fontSize, LocalConfiguration.current.locales[0])}") {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
            Divider()
            val bg by core.background.collectAsState()
            RowItem(stringResource(R.string.settings_background), icon = McIcons.Bell, onClick = { nav.go(Screen.Background) }, value = if (bg) stringResource(R.string.settings_background_on) else stringResource(R.string.settings_background_off)) {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
            Divider()
            val symbol by core.agentSymbol.collectAsState()
            RowItem(stringResource(R.string.settings_agents), icon = McIcons.Claude, onClick = { nav.go(Screen.Agents) }, value = listOf(stringResource(R.string.settings_agent_symbol_auto), stringResource(R.string.settings_agent_symbol_always), stringResource(R.string.settings_agent_symbol_never))[symbol.coerceIn(0, 2)]) {
                McIcon(McIcons.Chevron, c.muted, 16.dp)
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.settings_appearance))
            Block {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.settings_mode), style = McType.small, color = c.text)
                        Segmented(listOf(stringResource(R.string.settings_mode_system), stringResource(R.string.settings_mode_light), stringResource(R.string.settings_mode_dark)), mode.ordinal, { core.themeMode.set(ThemeMode.entries[it]) }, Modifier.fillMaxWidth())
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        Text(stringResource(R.string.settings_accent), style = McType.small, color = c.text)
                        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            AccentChoices.forEach { col ->
                                val sel = col == accent
                                val desc = stringResource(R.string.settings_accent_n, AccentChoices.indexOf(col) + 1)
                                Box(
                                    Modifier.size(44.dp).clip(RoundedCornerShape(22.dp)).then(if (sel) Modifier.border(2.dp, c.text, RoundedCornerShape(22.dp)) else Modifier)
                                        .selectable(sel, role = Role.RadioButton) { core.accent.set(col) }.semantics { contentDescription = desc },
                                    contentAlignment = Alignment.Center,
                                ) { Box(Modifier.size(30.dp).clip(RoundedCornerShape(15.dp)).background(col)) }
                            }
                        }
                    }
                }
            }
        }
        PathLineSettings(core)
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.settings_quick_keys))
            Block {
                listOf("start" to (stringResource(R.string.settings_bar_start) to startBar), "detail" to (stringResource(R.string.settings_terminal) to detailBar)).forEach { (which, info) ->
                    Column(Modifier.fillMaxWidth().clickable { nav.go(Screen.Bar(which)) }.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(info.first, style = McType.body, color = c.text, modifier = Modifier.weight(1f))
                            McIcon(McIcons.Chevron, c.muted, 16.dp)
                        }
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            info.second.mapNotNull(QuickKeys::get).forEach { KeyChip(stringResource(it.label)) }
                        }
                    }
                    Divider()
                }
                Text(stringResource(R.string.settings_quick_keys_hint),
                    style = McType.caption, color = c.muted, modifier = Modifier.padding(14.dp))
                Divider()
                val compactKeys by core.compactKeys.collectAsState()
                RowItem(stringResource(R.string.settings_height), stringResource(R.string.settings_height_hint)) {
                    Segmented(listOf(stringResource(R.string.settings_height_normal), stringResource(R.string.settings_height_compact)), if (compactKeys) 1 else 0, { core.compactKeys.set(it == 1) }, compact = true)
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.settings_input))
            val hideKb by core.hideKeyboardAfterSend.collectAsState()
            val suggest by core.keyboardSuggestions.collectAsState()
            Block {
                RowItem(stringResource(R.string.settings_hide_keyboard), stringResource(R.string.settings_hide_keyboard_hint)) {
                    McSwitch(hideKb, { core.hideKeyboardAfterSend.set(it) })
                }
                Divider()
                RowItem(stringResource(R.string.settings_suggestions), stringResource(R.string.settings_suggestions_hint)) {
                    McSwitch(suggest, { core.keyboardSuggestions.set(it) })
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.settings_navigation))
            Block {
                RowItem(stringResource(R.string.settings_show_dock), stringResource(R.string.settings_show_dock_hint)) {
                    McSwitch(dock, { core.showDock.set(it) })
                }
                Divider()
                val menuRight by core.menuRight.collectAsState()
                RowItem(stringResource(R.string.settings_menu_button), stringResource(R.string.settings_menu_button_hint)) {
                    Segmented(listOf(stringResource(R.string.settings_menu_left), stringResource(R.string.settings_menu_right)), if (menuRight) 1 else 0, { core.menuRight.set(it == 1) }, compact = true)
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.settings_about))
            Block {
                RowItem(stringResource(R.string.settings_version)) {
                    Text(BuildConfig.VERSION_NAME, style = McType.small, color = c.muted)
                }
                Divider()
                val demoOn by core.demo.collectAsState()
                RowItem(stringResource(R.string.settings_demo), stringResource(R.string.settings_demo_hint)) {
                    McSwitch(demoOn, { core.setDemo(it) })
                }
                Divider()
                RowItem(stringResource(R.string.licenses_title), stringResource(R.string.settings_licenses_hint), onClick = { nav.go(Screen.Licenses) }) {
                    McIcon(McIcons.Chevron, c.muted, 16.dp)
                }
            }
        }
    }
}

@Composable
private fun KeyChip(label: String, onRemove: (() -> Unit)? = null) {
    val c = LocalMc.current
    Row(Modifier.height(30.dp).clip(RoundedCornerShape(8.dp)).background(c.inset).padding(start = 8.dp, end = if (onRemove != null) 0.dp else 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = McType.path.copy(fontSize = TextUnit(12f, TextUnitType.Sp)), color = c.text)
        onRemove?.let { IconAction(McIcons.Close, stringResource(R.string.settings_remove_item, label), size = 30.dp, iconSize = 12.dp, onClick = it) }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun BarEditorScreen(core: Core, nav: Nav, which: String) {
    val c = LocalMc.current
    val current by (if (which == "detail") core.detailBar else core.startBar).collectAsState()
    val max = if (which == "detail") QuickKeys.MAX_DETAIL else QuickKeys.MAX_START
    val available = (if (which == "detail") QuickKeys.availableDetail else QuickKeys.availableStart).filterNot { it in current }
    Page(if (which == "detail") stringResource(R.string.settings_bar_detail_title) else stringResource(R.string.settings_bar_start_title), nav) {
        Text(
            if (which == "detail") stringResource(R.string.settings_bar_detail_count, current.size, max)
            else stringResource(R.string.settings_bar_start_count, current.size, max),
            style = McType.small, color = c.muted,
        )
        Block {
            current.forEachIndexed { i, id ->
                val k = QuickKeys.get(id) ?: return@forEachIndexed
                Row(Modifier.fillMaxWidth().height(52.dp).padding(start = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                    k.icon?.let { McIcon(it, c.text, 18.dp) }
                    // Next to an icon a sign like "↓" would only repeat it: then what the key does.
                    val label = stringResource(k.label)
                    val text = if (k.icon == null) label else if (label.any(Char::isLetter)) label.removePrefix("⏎ ") else stringResource(k.description)
                    Text(text, style = McType.body, color = c.text, modifier = Modifier.weight(1f).padding(start = 12.dp))
                    IconAction(McIcons.Up, stringResource(R.string.settings_move_up), iconSize = 16.dp) {
                        if (i > 0) core.bar(which).set(current.toMutableList().also { l -> l[i] = l[i - 1]; l[i - 1] = id })
                    }
                    IconAction(McIcons.Down, stringResource(R.string.settings_move_down), iconSize = 16.dp) {
                        if (i < current.lastIndex) core.bar(which).set(current.toMutableList().also { l -> l[i] = l[i + 1]; l[i + 1] = id })
                    }
                    IconAction(McIcons.Close, stringResource(R.string.settings_remove_item, stringResource(k.label)), iconSize = 14.dp) { core.bar(which).set(current - id) }
                }
                Divider()
            }
        }
        SectionTitle(stringResource(R.string.settings_add))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            available.mapNotNull(QuickKeys::get).forEach { k ->
                Row(
                    Modifier.height(40.dp).clip(RoundedCornerShape(10.dp)).border(1.dp, c.line, RoundedCornerShape(10.dp))
                        .clickable(enabled = current.size < max) { core.bar(which).set(current + k.id) }.padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) { McIcon(McIcons.Plus, c.muted, 12.dp); Text(stringResource(k.label), style = McType.path.copy(fontSize = TextUnit(13f, TextUnitType.Sp)), color = c.text) }
            }
        }
        SecondaryButton(stringResource(R.string.settings_restore_default), Modifier.fillMaxWidth()) { core.bar(which).set(core.settings.defaultBar(which)) }
    }
}

/** "SHA256:…" of an OpenSSH public key line, as `ssh-keygen -l` shows it; null if it is no such line. */
internal fun sshFingerprint(openssh: String): String? = runCatching {
    val blob = java.util.Base64.getDecoder().decode(openssh.trim().split(' ')[1])
    "SHA256:" + java.util.Base64.getEncoder().withoutPadding().encodeToString(java.security.MessageDigest.getInstance("SHA-256").digest(blob))
}.getOrNull()

@Composable
fun ServersScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val saved by core.servers.collectAsState()
    val state by core.real.collectAsState()
    Page(stringResource(R.string.settings_servers), nav) {
        Block {
            saved.forEach { s ->
                val conn = state.servers.firstOrNull { it.id == s.id }?.connection
                val online = conn is Connection.Online
                Row(Modifier.fillMaxWidth().clickable { nav.go(Screen.ServerEdit(s.id)) }.padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).then(if (online) Modifier.background(c.accentText) else Modifier.border(1.5.dp, c.muted, RoundedCornerShape(4.dp))))
                    Column(Modifier.weight(1f)) {
                        Text(s.name, style = McType.body, color = c.text)
                        EllipsisText("${s.user}@${s.host}" + if (s.port != 22) ":${s.port}" else "", McType.path, c.muted)
                    }
                    Text(conn?.let { connectionText(core, s.id, it) } ?: stringResource(R.string.settings_server_passive), style = McType.caption, color = if (online) c.text else c.muted)
                    McIcon(McIcons.Chevron, c.muted, 16.dp)
                }
                Divider()
            }
            Row(Modifier.fillMaxWidth().height(52.dp).clickable { nav.go(Screen.ServerEdit(null)) }.padding(horizontal = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                McIcon(McIcons.Plus, c.muted, 16.dp); Text(stringResource(R.string.settings_server_add), style = McType.body, color = c.muted)
            }
        }
    }
}

@Composable
fun ServerEditScreen(core: Core, nav: Nav, serverId: String?) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val saved by core.servers.collectAsState()
    val keys by core.keyList.collectAsState()
    val state by core.real.collectAsState()
    val existing = saved.firstOrNull { it.id == serverId }
    val id = remember { existing?.id ?: core.newServerId() }
    var name by remember { mutableStateOf(existing?.name ?: "") }
    var host by remember { mutableStateOf(existing?.host ?: "") }
    var port by remember { mutableStateOf((existing?.port ?: 22).toString()) }
    var user by remember { mutableStateOf(existing?.user ?: "") }
    var session by remember { mutableStateOf(existing?.session ?: "") }
    var usePassword by remember { mutableStateOf(existing != null && existing.keyId == null) }
    var keyId by remember { mutableStateOf(existing?.keyId ?: keys.firstOrNull()?.id) }
    // A key made in this form: show how it gets onto the server.
    var madeHere by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(keys) { if (keys.none { it.id == keyId }) keyId = keys.firstOrNull()?.id }
    var password by remember { mutableStateOf("") }
    var autoConnect by remember { mutableStateOf(existing?.autoConnect ?: true) }
    var notify by remember { mutableStateOf(existing?.notify ?: true) }
    var confirmRemove by remember { mutableStateOf(false) }
    val conn = state.servers.firstOrNull { it.id == id }?.connection

    Page(if (existing == null) stringResource(R.string.settings_server_new) else existing.name, nav) {
        if (conn != null) Text(connectionText(core, existing?.id.orEmpty(), conn), style = McType.small, color = if (conn is Connection.Online) c.accentText else c.muted)
        if (conn is Connection.HostKeyUnknown) {
            Block {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(stringResource(R.string.settings_hostkey_unknown), style = McType.bodyStrong, color = c.text)
                    Text(conn.fingerprint, style = McType.path, color = c.mono)
                    Text(stringResource(R.string.settings_hostkey_compare), style = McType.caption, color = c.muted)
                    PrimaryButton(stringResource(R.string.settings_hostkey_trust_connect), Modifier.fillMaxWidth()) { core.trust(id, conn) }
                }
            }
        }
        if (conn is Connection.HostKeyChanged) {
            Text(stringResource(R.string.settings_hostkey_changed, conn.fingerprint), style = McType.small, color = c.danger)
        }
        SectionTitle(stringResource(R.string.settings_connection))
        Block {
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                LabeledField(stringResource(R.string.settings_server_name), name, { name = it }, stringResource(R.string.settings_server_name_hint))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) { LabeledField(stringResource(R.string.settings_server_host), host, { host = it.filterNot(Char::isWhitespace) }, "server.example.com", mono = true, address = true) }
                    Box(Modifier.size(width = 84.dp, height = 70.dp)) { LabeledField(stringResource(R.string.settings_server_port), port, { port = it.filter(Char::isDigit) }, "22", mono = true, number = true) }
                }
                LabeledField(stringResource(R.string.settings_server_user), user, { user = it }, stringResource(R.string.settings_server_user_hint), mono = true, address = true)
                LabeledField(stringResource(R.string.settings_server_session), session, { session = it }, stringResource(R.string.settings_server_session_hint), mono = true, address = true)
            }
        }
        SectionTitle(stringResource(R.string.settings_login))
        KeyAdding(core, onAdded = { keyId = it.id; madeHere = it.id.takeIf { _ -> it.origin == KeyVault.Origin.GENERATED } }) { adder ->
        Block {
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Segmented(listOf(stringResource(R.string.settings_auth_key), stringResource(R.string.settings_auth_password)), if (usePassword) 1 else 0, { usePassword = it == 1 }, Modifier.fillMaxWidth())
                if (!usePassword && keys.isEmpty()) {
                    Text(stringResource(R.string.settings_no_key_yet), style = McType.small, color = c.muted)
                    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.inset)) { AddKeyRows(adder) }
                } else if (!usePassword) {
                    keys.forEach { k ->
                        Row(Modifier.fillMaxWidth().clickable { keyId = k.id }.padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Box(Modifier.size(18.dp).clip(RoundedCornerShape(9.dp)).border(2.dp, if (keyId == k.id) c.accentText else c.muted, RoundedCornerShape(9.dp)), contentAlignment = Alignment.Center) {
                                if (keyId == k.id) Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(c.accentText))
                            }
                            Column {
                                Text(k.name, style = McType.body, color = c.text)
                                Text(k.algorithm + if (k.fingerprint.isNotEmpty()) " · " + k.fingerprint.take(18) + "…" else "", style = McType.path, color = c.muted)
                            }
                        }
                    }
                    keys.firstOrNull { it.id == madeHere && it.id == keyId }?.let { k ->
                        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.inset).padding(12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            Text(stringResource(R.string.settings_key_to_server), style = McType.small, color = c.text)
                            Text(k.publicOpenssh, style = McType.path, color = c.muted)
                            SecondaryButton(stringResource(R.string.sshkeys_copy_public), background = c.surface) {
                                Attachments.copy(context, "SSH public key", k.publicOpenssh); context.toast(resources.getString(R.string.sshkeys_public_copied))
                            }
                        }
                    }
                    Text(stringResource(R.string.settings_manage_keys), style = McType.small, color = c.accentText, modifier = Modifier.clickable { nav.go(Screen.Keys) })
                } else {
                    LabeledField(stringResource(R.string.settings_auth_password), password, { password = it }, if (existing != null) stringResource(R.string.settings_password_unchanged) else "", password = true)
                    Text(stringResource(R.string.settings_password_hint), style = McType.caption, color = c.muted)
                }
            }
        }
        }
        SectionTitle(stringResource(R.string.settings_behavior))
        Block {
            RowItem(stringResource(R.string.settings_auto_connect), stringResource(R.string.settings_auto_connect_hint)) {
                McSwitch(autoConnect, { autoConnect = it })
            }
            Divider()
            RowItem(stringResource(R.string.settings_notifications), stringResource(R.string.settings_server_notify_hint)) {
                McSwitch(notify, { notify = it })
            }
        }
        if (existing != null) {
            SectionTitle(stringResource(R.string.settings_hostkey))
            Block {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    val trusted = core.settings.trustedKeys().filter { it.host.equals(existing.host, true) && it.port == existing.port }
                    Text(if (trusted.isEmpty()) stringResource(R.string.settings_hostkey_none) else trusted.joinToString("\n") { it.openssh.substringBefore(' ') + " · " + (sshFingerprint(it.openssh) ?: it.openssh.substringAfter(' ').take(24) + "…") }, style = McType.path, color = c.mono)
                    if (trusted.isNotEmpty()) SecondaryButton(stringResource(R.string.settings_hostkey_revoke)) { core.forgetHostKey(existing) }
                }
            }
        }
        PrimaryButton(stringResource(R.string.settings_save), Modifier.fillMaxWidth()) {
            if (host.isBlank() || user.isBlank()) return@PrimaryButton
            if (!usePassword && keys.none { it.id == keyId }) { context.toast(resources.getString(R.string.settings_need_key)); return@PrimaryButton }
            val server = SavedServer(
                id = id, name = name.ifBlank { host }, host = host.trim(), port = port.toIntOrNull() ?: 22, user = user.trim(),
                session = session.ifBlank { null }, keyId = if (usePassword) null else keyId, autoConnect = autoConnect, notify = notify,
            )
            scope.launch {
                runCatching { core.saveServer(server, if (usePassword && password.isNotEmpty()) password else null) }
                    .onSuccess { nav.back() }.onFailure { context.toast(it.message, long = true) }
            }
        }
        if (existing != null) SecondaryButton(stringResource(R.string.settings_server_remove), Modifier.fillMaxWidth(), color = c.danger) { confirmRemove = true }
    }
    if (confirmRemove && existing != null) {
        ConfirmDialog(stringResource(R.string.settings_server_remove_title, existing.name), stringResource(R.string.settings_server_remove_text), stringResource(R.string.settings_remove),
            onConfirm = { confirmRemove = false; core.removeServer(existing.id); nav.back() }, onDismiss = { confirmRemove = false })
    }
}

private val SpeechLanguages = listOf(
    "de-CH" to R.string.settings_speech_lang_de_ch, "de-DE" to R.string.settings_speech_lang_de, "en-US" to R.string.settings_speech_lang_en,
    "fr-CH" to R.string.settings_speech_lang_fr, "it-CH" to R.string.settings_speech_lang_it, "" to R.string.settings_speech_lang_system,
)

@Composable
fun SpeechScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val speech by core.speech.collectAsState()
    val services = remember { SpeechText.services(context) }
    val onDevice = remember { Build.VERSION.SDK_INT >= 31 && SpeechRecognizer.isOnDeviceRecognitionAvailable(context) }
    var wordsDraft by remember { mutableStateOf(speech.vocabulary.joinToString(", ")) }
    var from by remember { mutableStateOf("") }
    var to by remember { mutableStateOf("") }
    fun toast(m: String) = context.toast(m)

    Page(stringResource(R.string.settings_speech), nav) {
        SectionTitle(stringResource(R.string.settings_speech_recognition))
        Block {
            ChoiceRow(stringResource(R.string.settings_speech_engine_device), if (onDevice) stringResource(R.string.settings_speech_device_hint) else stringResource(R.string.settings_speech_device_unavailable), speech.engine == "device") {
                core.speech.set(speech.copy(engine = "device"))
            }
            services.forEach { (component, label) ->
                Divider()
                ChoiceRow(label, stringResource(R.string.settings_speech_service_hint), speech.engine == "service:$component") {
                    core.speech.set(speech.copy(engine = "service:$component"))
                }
            }
            Divider()
            val installed by core.whisper.installed.collectAsState()
            val model = WhisperStore.info(speech.whisperModel)?.takeIf { it.id in installed }
            ChoiceRow(stringResource(R.string.settings_speech_whisper), model?.let { stringResource(R.string.settings_speech_whisper_model_hint, it.label(LocalContext.current)) } ?: stringResource(R.string.settings_speech_whisper_hint), speech.engine == "whisper") {
                val pick = model?.id ?: WhisperStore.MODELS.lastOrNull { it.id in installed }?.id ?: speech.whisperModel
                core.speech.set(speech.copy(engine = "whisper", whisperModel = pick))
            }
        }
        // How a take runs: text while speaking (Whisper only) and when it ends, together.
        SectionTitle(stringResource(R.string.settings_speech_recording))
        Block {
            if (speech.engine == "whisper") {
                RowItem(stringResource(R.string.settings_speech_whisper_live), stringResource(R.string.settings_speech_whisper_live_hint)) {
                    McSwitch(speech.whisperLive, { core.speech.set(speech.copy(whisperLive = it)) })
                }
                Divider()
            }
            RowItem(stringResource(R.string.settings_speech_stop), stringResource(R.string.settings_speech_stop_hint)) {
                val choices = SpeechPrefs.STOP_CHOICES
                Segmented(
                    choices.map { ms -> if (ms == 0) "✓" else "%s s".format(java.text.NumberFormat.getInstance().apply { maximumFractionDigits = 1 }.format(ms / 1000.0)) },
                    choices.indexOf(speech.stopAfterMs).coerceAtLeast(0), { core.speech.set(speech.copy(stopAfterMs = choices[it])) }, compact = true,
                )
            }
        }
        // Models only matter for Whisper.
        if (speech.engine == "whisper") WhisperModels(core)
        SectionTitle(stringResource(R.string.language))
        Block {
            SpeechLanguages.forEachIndexed { i, (tag, label) ->
                if (i > 0) Divider()
                ChoiceRow(stringResource(label), if (tag == "de-CH") stringResource(R.string.settings_speech_lang_de_ch_hint, tag) else tag.ifEmpty { null }, speech.language == tag) { core.speech.set(speech.copy(language = tag)) }
            }
        }
        SectionTitle(stringResource(R.string.settings_speech_words))
        Block {
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(stringResource(R.string.settings_speech_words_hint), style = McType.caption, color = c.muted)
                Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).border(1.dp, c.line, RoundedCornerShape(10.dp)).background(c.bg).padding(12.dp)) {
                    PrivateTextField(wordsDraft, { wordsDraft = it }, "Claude Code, herdr, Proxmox …", Modifier.fillMaxWidth(), singleLine = false)
                }
                SecondaryButton(stringResource(R.string.settings_apply)) {
                    core.speech.set(speech.copy(vocabulary = wordsDraft.split(',').map { it.trim() }.filter { it.isNotEmpty() })); toast(resources.getString(R.string.settings_speech_words_saved))
                }
            }
        }
        SectionTitle(stringResource(R.string.settings_speech_replacements))
        Block {
            speech.replacements.forEachIndexed { i, (f, t) ->
                Row(Modifier.fillMaxWidth().padding(start = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(f, style = McType.path, color = c.text, modifier = Modifier.weight(1f))
                    McIcon(McIcons.Right, c.muted, 14.dp)
                    Text(t, style = McType.path, color = c.text, modifier = Modifier.weight(1f))
                    IconAction(McIcons.Close, stringResource(R.string.settings_speech_replacement_remove), iconSize = 14.dp) {
                        core.speech.set(speech.copy(replacements = speech.replacements.filterIndexed { j, _ -> j != i }))
                    }
                }
                Divider()
            }
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) { LabeledField(stringResource(R.string.settings_speech_recognized), from, { from = it }, stringResource(R.string.settings_speech_recognized_hint)) }
                    Box(Modifier.weight(1f)) { LabeledField(stringResource(R.string.settings_speech_replace_with), to, { to = it }, "herdr") }
                }
                SecondaryButton(stringResource(R.string.settings_add), Modifier.fillMaxWidth()) {
                    if (from.isNotBlank()) { core.speech.set(speech.copy(replacements = speech.replacements + (from.trim() to to.trim()))); from = ""; to = "" }
                }
                SecondaryButton(stringResource(R.string.settings_speech_from_voxtype), Modifier.fillMaxWidth()) {
                    val clip = Attachments.clipboard(context)
                    if (clip is ClipContent.Text) {
                        val (words, pairs) = SpeechText.fromVoxtype(clip.text)
                        if (words.isEmpty() && pairs.isEmpty()) toast(resources.getString(R.string.settings_speech_voxtype_none))
                        else {
                            val mergedWords = (speech.vocabulary + words).distinct()
                            val mergedPairs = (speech.replacements + pairs).distinctBy { it.first.lowercase() }
                            core.speech.set(speech.copy(vocabulary = mergedWords, replacements = mergedPairs))
                            wordsDraft = mergedWords.joinToString(", ")
                            toast(resources.getString(R.string.settings_speech_voxtype_imported,
                                resources.getQuantityString(R.plurals.settings_speech_words_count, words.size, words.size),
                                resources.getQuantityString(R.plurals.settings_speech_replacements_count, pairs.size, pairs.size)))
                        }
                    } else toast(resources.getString(R.string.settings_speech_voxtype_copy_first))
                }
            }
        }
        SectionTitle(stringResource(R.string.settings_speech_postprocessing))
        Block {
            RowItem(stringResource(R.string.settings_speech_auto_send), stringResource(R.string.settings_speech_auto_send_hint)) {
                McSwitch(speech.autoSend, { core.speech.set(speech.copy(autoSend = it)) })
            }
        }
    }
}

/** Whisper models: download, choose, delete, import. */
@Composable
private fun WhisperModels(core: Core) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val speech by core.speech.collectAsState()
    val installed by core.whisper.installed.collectAsState()
    val downloads by core.whisper.downloads.collectAsState()
    var confirmDelete by remember { mutableStateOf<WhisperModelInfo?>(null) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) scope.launch {
            runCatching { core.whisper.import(uri) }
                .onSuccess { context.toast(resources.getString(R.string.settings_whisper_imported, it)) }
                .onFailure { context.toast(it.message, long = true) }
        }
    }
    fun mb(b: Long) = if (b >= 1_000_000_000) "%.1f GB".format(b / 1e9) else "${b / 1_000_000} MB"

    SectionTitle(stringResource(R.string.settings_whisper_models))
    Block {
        WhisperStore.MODELS.forEachIndexed { i, m ->
            if (i > 0) Divider()
            val have = m.id in installed
            val dl = downloads[m.id]
            val chosen = have && speech.engine == "whisper" && speech.whisperModel == m.id
            Row(
                Modifier.fillMaxWidth().clickable(enabled = have, role = Role.RadioButton) { core.speech.set(speech.copy(engine = "whisper", whisperModel = m.id)) }
                    .padding(start = 14.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(m.label(LocalContext.current), style = McType.body, color = c.text)
                    Text(
                        when (dl) {
                            is WhisperStore.Download.Running -> stringResource(R.string.settings_whisper_downloading, dl.done * 100 / dl.total.coerceAtLeast(1), mb(m.bytes))
                            is WhisperStore.Download.Failed -> stringResource(R.string.settings_whisper_error, dl.res?.let { stringResource(it) } ?: dl.detail.orEmpty())
                            null -> "${mb(m.bytes)} · ${stringResource(m.noteRes)}"
                        },
                        style = McType.caption, color = if (dl is WhisperStore.Download.Failed) c.danger else c.muted,
                    )
                }
                when {
                    dl is WhisperStore.Download.Running ->
                        IconAction(McIcons.Close, stringResource(R.string.settings_whisper_cancel_download), iconSize = 14.dp) { core.whisper.cancel(m.id) }
                    have -> {
                        if (chosen) McIcon(McIcons.Check, c.accentText, 18.dp)
                        IconAction(McIcons.Trash, stringResource(R.string.settings_whisper_delete_desc, m.label(LocalContext.current)), iconSize = 16.dp) { confirmDelete = m }
                    }
                    else -> Text(stringResource(R.string.settings_whisper_download), style = McType.small, color = c.accentText,
                        modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { core.whisper.download(m) }.padding(horizontal = 10.dp, vertical = 12.dp))
                }
            }
        }
        Divider()
        RowItem(stringResource(R.string.settings_whisper_from_file), stringResource(R.string.settings_whisper_from_file_hint), McIcons.File, onClick = { pick.launch(arrayOf("*/*")) })
    }
    Text(
        stringResource(R.string.settings_whisper_note),
        style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp),
    )
    confirmDelete?.let { m ->
        ConfirmDialog(stringResource(R.string.settings_whisper_delete_title, m.label(LocalContext.current)), stringResource(R.string.settings_whisper_delete_text, mb(m.bytes)), stringResource(R.string.settings_delete),
            onConfirm = {
                confirmDelete = null
                core.whisper.delete(m.id)
                if (speech.whisperModel == m.id && speech.engine == "whisper") core.speech.set(speech.copy(engine = "device"))
            }, onDismiss = { confirmDelete = null })
    }
}

/** A radio row with title and explanation (speech engines, key protection). */
@Composable
internal fun ChoiceRow(title: String, subtitle: String?, selected: Boolean, enabled: Boolean = true, badge: String? = null, onClick: () -> Unit) {
    val c = LocalMc.current
    Row(
        Modifier.fillMaxWidth().selectable(selected, enabled = enabled, role = Role.RadioButton, onClick = onClick).padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(Modifier.size(18.dp).clip(RoundedCornerShape(9.dp)).border(2.dp, if (selected) c.accentText else c.muted, RoundedCornerShape(9.dp)), contentAlignment = Alignment.Center) {
            if (selected) Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(c.accentText))
        }
        Column(Modifier.weight(1f)) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(title, style = McType.body, color = if (enabled) c.text else c.muted, modifier = Modifier.alignByBaseline())
                badge?.let { Text("· $it", style = McType.caption, color = c.muted, modifier = Modifier.alignByBaseline()) }
            }
            subtitle?.let { Text(it, style = McType.caption, color = c.muted) }
        }
    }
}

private const val SAMPLE = "\u001B[1m⏺ Bash\u001B[0m(npm test)\n  ⎿  \u001B[32mPASS\u001B[0m  stats/ranking.test.ts\n  ⎿  \u001B[31mFAIL\u001B[0m  stats/history.test.ts\n\u001B[33mwarning\u001B[0m: \u001B[36munused\u001B[0m variable \u001B[35m`x`\u001B[0m\n\u001B[34m❯\u001B[0m 1. Yes   \u001B[90m2. No\u001B[0m"

/** Terminal font size in half steps, with the preset names as orientation. */
@Composable
private fun FontSizeStepper(size: Float, onChange: (Float) -> Unit) {
    val c = LocalMc.current
    Block {
        Row(Modifier.fillMaxWidth().padding(horizontal = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(McIcons.Minus, stringResource(R.string.settings_font_smaller), if (size > TerminalPrefs.MIN_FONT) c.text else c.line) {
                if (size > TerminalPrefs.MIN_FONT) onChange(size - 0.5f)
            }
            Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(TerminalPrefs.sizeLabel(size, LocalConfiguration.current.locales[0]), style = McType.bodyStrong, color = c.text)
                Text(stringResource(TerminalPrefs.sizeNameRes(size)), style = McType.caption, color = c.muted)
            }
            IconAction(McIcons.Plus, stringResource(R.string.settings_font_larger), if (size < TerminalPrefs.MAX_FONT) c.text else c.line) {
                if (size < TerminalPrefs.MAX_FONT) onChange(size + 0.5f)
            }
        }
    }
}

@Composable
private fun SchemePreview(scheme: TerminalScheme, fontSize: Float) {
    val lines = remember(scheme) { Ansi.lines(SAMPLE, scheme) }
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(scheme.bg()).padding(14.dp)) {
        lines.forEach { Text(it, style = McType.output.copy(fontSize = TextUnit(fontSize, TextUnitType.Sp)), color = scheme.fg()) }
    }
}

@Composable
private fun Swatches(scheme: TerminalScheme) {
    Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
        Box(Modifier.size(14.dp).clip(RoundedCornerShape(3.dp)).background(scheme.bg()).border(1.dp, LocalMc.current.line, RoundedCornerShape(3.dp)))
        (1..6).forEach { Box(Modifier.size(14.dp).clip(RoundedCornerShape(3.dp)).background(scheme.color(it))) }
    }
}

private fun hex(v: Long) = hexRgb(v.toInt())
private fun parseHex(s: String): Long? = parseRgb(s)?.let { 0xFF000000 or it.toLong() }

@Composable
fun TerminalScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val term by core.terminal.collectAsState()
    var editing by remember { mutableStateOf<String?>(null) }
    val current = term.scheme(c.dark)

    val edited = term.custom.firstOrNull { it.id == editing }
    if (edited != null) {
        SchemeEditor(core, nav, term, edited) { editing = null }
        return
    }
    Page(stringResource(R.string.settings_terminal), nav) {
        SectionTitle(stringResource(R.string.settings_view))
        Segmented(listOf(stringResource(R.string.settings_term_view_normal), "TUI"), if (term.tui) 1 else 0, { core.terminal.set(term.copy(tui = it == 1)) })
        // Both described all the time, so the choice is made knowing the TUI's drawback.
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(
                stringResource(R.string.settings_term_view_normal) to stringResource(R.string.settings_term_normal_desc),
                "TUI" to stringResource(R.string.settings_term_tui_desc),
                stringResource(R.string.settings_term_tui_drawback) to stringResource(R.string.settings_term_tui_drawback_desc),
            ).forEach { (title, text) ->
                Column {
                    Text(title, style = McType.small, color = c.text)
                    Text(text, style = McType.caption, color = c.muted)
                }
            }
        }
        if (term.tui) {
            SectionTitle(stringResource(R.string.settings_term_linger))
            val choices = listOf(
                0 to stringResource(R.string.settings_term_linger_off), 10 to stringResource(R.string.settings_term_linger_seconds, 10),
                20 to stringResource(R.string.settings_term_linger_seconds, 20), 60 to stringResource(R.string.settings_term_linger_minutes, 1),
            )
            Segmented(choices.map { it.second }, choices.indexOfFirst { it.first == term.lingerSeconds }.coerceAtLeast(0),
                { core.terminal.set(term.copy(lingerSeconds = choices[it].first)) })
            Text(
                stringResource(R.string.settings_term_linger_desc),
                style = McType.caption, color = c.muted,
            )
        }
        SchemePreview(current, term.fontSize)
        SectionTitle(stringResource(R.string.settings_font_size))
        FontSizeStepper(term.fontSize) { core.terminal.set(term.copy(fontSize = it)) }
        SectionTitle(stringResource(R.string.settings_color_scheme))
        Block {
            SchemeRow(stringResource(R.string.settings_scheme_auto), term.schemeId == TerminalSchemes.AUTO, if (c.dark) TerminalSchemes.MissionDark else TerminalSchemes.MissionLight,
                onPick = { core.terminal.set(term.copy(schemeId = TerminalSchemes.AUTO)) }, onCopy = null)
            (term.custom + TerminalSchemes.presets).forEach { sch ->
                Divider()
                // A preset's name in the app's language; a copy keeps it as its own (custom) name.
                val shown = sch.displayName()
                val ownName = stringResource(R.string.settings_scheme_own_name, shown)
                SchemeRow(shown, term.schemeId == sch.id, sch,
                    onPick = { core.terminal.set(term.copy(schemeId = sch.id)) },
                    onCopy = {
                        val copy = sch.copy(id = "custom-" + System.currentTimeMillis(), name = if (sch.custom) sch.name else ownName, custom = true)
                        core.terminal.set(term.copy(custom = term.custom + copy, schemeId = copy.id)); editing = copy.id
                    },
                    onEdit = if (sch.custom) ({ editing = sch.id }) else null)
            }
        }
        Text(stringResource(R.string.settings_scheme_hint), style = McType.caption, color = c.muted)
    }
}

@Composable
private fun SchemeRow(name: String, selected: Boolean, scheme: TerminalScheme, onPick: () -> Unit, onCopy: (() -> Unit)?, onEdit: (() -> Unit)? = null) {
    val c = LocalMc.current
    Row(Modifier.fillMaxWidth().clickable(role = Role.RadioButton, onClick = onPick).padding(start = 14.dp, top = 6.dp, bottom = 6.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(18.dp).clip(RoundedCornerShape(9.dp)).border(2.dp, if (selected) c.accentText else c.muted, RoundedCornerShape(9.dp)), contentAlignment = Alignment.Center) {
            if (selected) Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(c.accentText))
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(name, style = McType.body, color = c.text)
            Swatches(scheme)
        }
        onEdit?.let { IconAction(McIcons.Pencil, stringResource(R.string.settings_scheme_edit, name), iconSize = 16.dp, onClick = it) }
        onCopy?.let { IconAction(McIcons.Copy, stringResource(R.string.settings_scheme_copy, name), iconSize = 16.dp, onClick = it) }
    }
}

@Composable
private fun SchemeEditor(core: Core, nav: Nav, term: TerminalPrefs, scheme: TerminalScheme, onDone: () -> Unit) {
    val c = LocalMc.current
    var name by remember(scheme.id) { mutableStateOf(scheme.name) }
    val values = remember(scheme.id) { mutableStateListOf<String>().apply { add(hex(scheme.background)); add(hex(scheme.foreground)); scheme.ansi.forEach { add(hex(it)) } } }
    fun build(): TerminalScheme = scheme.copy(
        name = name.ifBlank { scheme.name },
        background = parseHex(values[0]) ?: scheme.background,
        foreground = parseHex(values[1]) ?: scheme.foreground,
        ansi = (0 until 16).map { parseHex(values[it + 2]) ?: scheme.ansi[it] },
    )
    var confirmDelete by remember { mutableStateOf(false) }
    BackHandler { onDone() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(McIcons.Back, stringResource(R.string.settings_back), c.text) { onDone() }
            Text(stringResource(R.string.settings_scheme_customize), style = McType.title, color = c.text)
        }
        SchemePreview(build(), term.fontSize)
        LabeledField(stringResource(R.string.settings_scheme_name), name, { name = it })
        Block {
            (listOf(stringResource(R.string.settings_scheme_background), stringResource(R.string.settings_scheme_foreground)) + TerminalSchemes.colorNameRes.map { stringResource(it) }).forEachIndexed { i, label ->
                Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Box(Modifier.size(24.dp).clip(RoundedCornerShape(6.dp)).background(Color(parseHex(values[i]) ?: 0xFF000000)).border(1.dp, c.line, RoundedCornerShape(6.dp)))
                    Text(label, style = McType.small, color = c.text, modifier = Modifier.weight(1f))
                    Box(Modifier.size(width = 110.dp, height = 36.dp).clip(RoundedCornerShape(8.dp)).border(1.dp, c.line, RoundedCornerShape(8.dp)).background(c.bg).padding(horizontal = 8.dp), contentAlignment = Alignment.CenterStart) {
                        PrivateTextField(values[i], { values[i] = it }, "#RRGGBB", Modifier.fillMaxWidth(), mono = true)
                    }
                }
                if (i < 17) Divider()
            }
        }
        PrimaryButton(stringResource(R.string.settings_save), Modifier.fillMaxWidth()) {
            val updated = build()
            core.terminal.set(term.copy(custom = term.custom.map { if (it.id == scheme.id) updated else it }, schemeId = scheme.id)); onDone()
        }
        SecondaryButton(stringResource(R.string.settings_scheme_delete), Modifier.fillMaxWidth(), color = c.danger) { confirmDelete = true }
    }
    if (confirmDelete) {
        ConfirmDialog(stringResource(R.string.settings_scheme_delete_title, scheme.name), stringResource(R.string.settings_scheme_delete_text), stringResource(R.string.settings_delete),
            onConfirm = {
                confirmDelete = false
                core.terminal.set(term.copy(custom = term.custom.filterNot { it.id == scheme.id }, schemeId = if (term.schemeId == scheme.id) TerminalSchemes.AUTO else term.schemeId))
                onDone()
            }, onDismiss = { confirmDelete = false })
    }
}

/** Background connection and notifications. */
@Composable
fun BackgroundScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val context = LocalContext.current
    val background by core.background.collectAsState()
    val waiting by core.notifyWaiting.collectAsState()
    val done by core.notifyDone.collectAsState()
    fun hasNotifyPermission() = notificationsAllowed(context)
    var notifyAllowed by remember { mutableStateOf(hasNotifyPermission()) }
    val power = context.getSystemService(PowerManager::class.java)
    var unrestricted by remember { mutableStateOf(power.isIgnoringBatteryOptimizations(context.packageName)) }
    val askNotify = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { notifyAllowed = it }
    val notifySettings = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { notifyAllowed = hasNotifyPermission() }
    val askBattery = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        unrestricted = power.isIgnoringBatteryOptimizations(context.packageName)
    }
    var statusHidden by remember { mutableStateOf(Notifications.statusHidden(context)) }
    val channelSettings = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { statusHidden = Notifications.statusHidden(context) }
    Page(stringResource(R.string.settings_background), nav) {
        Block {
            RowItem(stringResource(R.string.settings_bg_stay_connected), stringResource(R.string.settings_bg_stay_connected_hint)) {
                McSwitch(background, { on ->
                    core.background.set(on)
                    if (on) {
                        if (!notifyAllowed && Build.VERSION.SDK_INT >= 33) askNotify.launch(Manifest.permission.POST_NOTIFICATIONS)
                        FleetService.start(context)
                    } else FleetService.stop(context)
                })
            }
        }
        if (background) {
            Block {
                RowItem(
                    if (statusHidden) stringResource(R.string.settings_bg_status_hidden) else stringResource(R.string.settings_bg_hide_status),
                    if (statusHidden) stringResource(R.string.settings_bg_status_hidden_hint) else stringResource(R.string.settings_bg_hide_status_hint),
                    onClick = {
                        Notifications.channels(context)
                        channelSettings.launch(
                            Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
                                .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                                .putExtra(Settings.EXTRA_CHANNEL_ID, Notifications.CHANNEL_STATUS),
                        )
                    },
                ) { McIcon(McIcons.Chevron, c.muted, 16.dp) }
            }
        }
        SectionTitle(stringResource(R.string.settings_notifications))
        Block {
            RowItem(stringResource(R.string.settings_notify_waiting), stringResource(R.string.settings_notify_waiting_hint)) {
                McSwitch(waiting, { core.notifyWaiting.set(it) })
            }
            Divider()
            RowItem(stringResource(R.string.settings_notify_done)) { McSwitch(done, { core.notifyDone.set(it) }) }
            Divider()
            val lost by core.notifyConnection.collectAsState()
            RowItem(stringResource(R.string.settings_notify_connection), stringResource(R.string.settings_notify_connection_hint)) {
                McSwitch(lost, { core.notifyConnection.set(it) })
            }
        }
        Text(
            stringResource(R.string.settings_notify_reply_hint),
            style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp),
        )
        if (background && !notifyAllowed) {
            Block {
                RowItem(stringResource(R.string.settings_notify_not_allowed), stringResource(R.string.settings_notify_not_allowed_hint), onClick = {
                    // Before Android 13 there is no permission to ask for, only the switch in the system settings.
                    if (Build.VERSION.SDK_INT >= 33) askNotify.launch(Manifest.permission.POST_NOTIFICATIONS)
                    else notifySettings.launch(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                }) { Text(stringResource(R.string.settings_allow), style = McType.small, color = c.accentText) }
            }
        }
        SectionTitle(stringResource(R.string.settings_battery))
        Block {
            RowItem(
                if (unrestricted) stringResource(R.string.settings_battery_off) else stringResource(R.string.settings_battery_turn_off),
                stringResource(R.string.settings_battery_hint),
                onClick = if (unrestricted) null else {
                    {
                        // Android's list, no permission needed (Play allows asking directly only for few apps).
                        askBattery.launch(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                    }
                },
            ) { if (!unrestricted) Text(stringResource(R.string.settings_turn_off), style = McType.small, color = c.accentText) }
        }
        Text(
            stringResource(R.string.settings_bg_privacy_note),
            style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp),
        )
    }
}
