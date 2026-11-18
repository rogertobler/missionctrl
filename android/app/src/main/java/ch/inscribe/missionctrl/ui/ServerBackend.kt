package ch.inscribe.missionctrl.ui

import android.content.res.Resources
import androidx.compose.ui.platform.LocalResources
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.SavedServer
import ch.inscribe.missionctrl.core.DetectedBackend

/** What the server form found out about a server's herdr, before saving it. */
sealed interface Detection {
    data object Checking : Detection
    data class Found(val backends: List<DetectedBackend>) : Detection
    data object None : Detection
    /** No clear answer (a probe timed out, the server could not be reached); [reason] is the error, if any. */
    data class Unclear(val reason: String? = null) : Detection
    data class HostKeyUnknown(val host: String, val port: Int, val fingerprint: String, val openssh: String) : Detection
    data class HostKeyChanged(val fingerprint: String) : Detection
    data object AuthFailed : Detection
}

/** What the server form does with a detection result. */
sealed interface DetectOutcome {
    /** Save as asked (only one herdr running, the type stays as chosen). */
    data object Save : DetectOutcome
    /** Both herdr: the user picks one or both. */
    data object AskWhich : DetectOutcome
    /** Only an installed, stopped herdr-core: saving as herdr-core would start it, so ask first. */
    data object AskStartCore : DetectOutcome
    /** Nothing to save yet; the status line explains. */
    data object Stay : DetectOutcome
}

object Backends {
    /**
     * Whether saving the form looks for herdr first: only with type "auto", and for an
     * existing server only when its address changed (it keeps what it found).
     */
    fun mustDetect(existing: SavedServer?, backend: String, host: String, port: String, user: String, session: String): Boolean =
        backend == SavedServer.BACKEND_AUTO && (existing == null ||
            existing.host != host.trim() || existing.port != (port.toIntOrNull() ?: 22) || existing.user != user.trim() ||
            existing.session != session.ifBlank { null } || existing.backend != SavedServer.BACKEND_AUTO)

    /**
     * Whether a detection that ran on [ran] (with password [ranPassword]) still applies to the
     * form as it is now ([now], [nowPassword]): same address, user, session and login. Checked
     * right before saving, because the form may change in the same frame the result arrives.
     */
    fun stillMatches(ran: SavedServer, ranPassword: String?, now: SavedServer?, nowPassword: String?): Boolean =
        now != null && now.host == ran.host && now.port == ran.port && now.user == ran.user &&
            now.session == ran.session && now.keyId == ran.keyId && nowPassword == ranPassword

    /** The status line for a finished detection: unclear when any probe gave no clear answer. */
    fun status(found: List<DetectedBackend>): Detection = when {
        found.any { it.unclear } -> Detection.Unclear()
        found.isEmpty() -> Detection.None
        else -> Detection.Found(found)
    }

    /**
     * The next step for [found], the result of `detectBackends` with type "auto". Any probe
     * without a clear answer stops here: nothing is saved on half a picture (the TUI may only
     * be slow, so asking to start herdr-core or saying "nothing found" could both be wrong).
     */
    fun outcome(found: List<DetectedBackend>): DetectOutcome {
        if (found.any { it.unclear }) return DetectOutcome.Stay
        val tui = found.any { it.backend == SavedServer.BACKEND_TUI }
        val core = found.firstOrNull { it.backend == SavedServer.BACKEND_CORE }
        return when {
            tui && core != null -> DetectOutcome.AskWhich
            core != null && !core.running -> DetectOutcome.AskStartCore
            tui || core != null -> DetectOutcome.Save
            else -> DetectOutcome.Stay
        }
    }

    /**
     * Short tag after a server's name, only where the same host is listed twice
     * (one entry per herdr): the saved type, else the one [connected] reports ("tui"/"core").
     */
    fun tagged(servers: List<SavedServer>, connected: (String) -> String?): Map<String, String?> {
        val twice = servers.groupBy { Triple(it.host.lowercase(), it.port, it.user) }.filterValues { it.size > 1 }.values.flatten().map { it.id }.toSet()
        return servers.associate { s ->
            s.id to if (s.id in twice) s.backend.takeIf { it != SavedServer.BACKEND_AUTO } ?: connected(s.id) else null
        }
    }
}

@Composable
fun backendName(backend: String): String = when (backend) {
    SavedServer.BACKEND_TUI -> stringResource(R.string.settings_backend_tui)
    SavedServer.BACKEND_CORE -> stringResource(R.string.settings_backend_core)
    else -> stringResource(R.string.settings_backend_auto)
}

/** "devbox · Core" where the host is listed twice, else just the name. */
@Composable
fun serverLabel(name: String, tag: String?): String = serverLabel(LocalResources.current, name, tag)

fun serverLabel(res: Resources, name: String, tag: String?): String = when (tag) {
    SavedServer.BACKEND_TUI -> res.getString(R.string.server_with_backend, name, res.getString(R.string.backend_short_tui))
    SavedServer.BACKEND_CORE -> res.getString(R.string.server_with_backend, name, res.getString(R.string.backend_short_core))
    else -> name
}

/** Status line under the type switch. */
@Composable
fun DetectionStatus(detection: Detection, onTrust: (Detection.HostKeyUnknown) -> Unit) {
    val c = LocalMc.current
    when (detection) {
        Detection.Checking -> Text(stringResource(R.string.settings_detect_checking), style = McType.small, color = c.muted)
        is Detection.Found -> {
            val names = detection.backends.map { it.backend }.distinct()
            Text(
                if (names.size > 1) stringResource(R.string.settings_detect_found_both) else stringResource(R.string.settings_detect_found, backendName(names.first())),
                style = McType.small, color = c.accentText,
            )
        }
        Detection.None -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.settings_detect_none_title), style = McType.bodyStrong, color = c.text)
            Text(stringResource(R.string.settings_detect_none_text), style = McType.small, color = c.muted)
        }
        is Detection.Unclear -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.settings_detect_unclear), style = McType.small, color = c.danger)
            detection.reason?.let { Text(it, style = McType.caption, color = c.muted) }
        }
        is Detection.HostKeyUnknown -> Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(stringResource(R.string.settings_hostkey_unknown), style = McType.bodyStrong, color = c.text)
            Text(detection.fingerprint, style = McType.path, color = c.mono)
            Text(stringResource(R.string.settings_hostkey_compare), style = McType.caption, color = c.muted)
            PrimaryButton(stringResource(R.string.settings_hostkey_trust_connect), Modifier.fillMaxWidth()) { onTrust(detection) }
        }
        is Detection.HostKeyChanged -> Text(stringResource(R.string.settings_hostkey_changed, detection.fingerprint), style = McType.small, color = c.danger)
        Detection.AuthFailed -> Text(stringResource(R.string.workspaces_conn_auth_failed), style = McType.small, color = c.danger)
    }
}

/** Only herdr-core found, installed but stopped: saving it as herdr-core starts it, so ask. */
@Composable
fun StartCoreDialog(onSave: () -> Unit, onDismiss: () -> Unit) {
    val c = LocalMc.current
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        title = { Text(stringResource(R.string.settings_detect_found, stringResource(R.string.settings_backend_core)), style = McType.title, color = c.text) },
        text = { Text(stringResource(R.string.settings_detect_core_installed), style = McType.small, color = c.muted) },
        confirmButton = { TextButton(onClick = onSave) { Text(stringResource(R.string.settings_save), style = McType.body, color = c.accentText) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.common_cancel), style = McType.body, color = c.text) } },
    )
}

/** Both herdr on one server: herdr TUI, herdr-core or both (two entries). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TwoHerdrSheet(name: String, found: List<DetectedBackend>, onSave: (List<String>) -> Unit, onDismiss: () -> Unit) {
    val c = LocalMc.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var choice by remember { mutableIntStateOf(0) }
    val coreRunning = found.firstOrNull { it.backend == SavedServer.BACKEND_CORE }?.running == true
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = c.bg) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.settings_two_herdr_title, name), style = McType.title, color = c.text)
            Text(stringResource(R.string.settings_two_herdr_text), style = McType.small, color = c.muted)
            Block {
                ChoiceRow(stringResource(R.string.settings_backend_tui), stringResource(R.string.settings_two_herdr_tui_sub), choice == 0) { choice = 0 }
                Divider()
                ChoiceRow(
                    stringResource(R.string.settings_backend_core),
                    stringResource(if (coreRunning) R.string.settings_two_herdr_core_sub else R.string.settings_two_herdr_core_stopped_sub),
                    choice == 1,
                ) { choice = 1 }
                Divider()
                ChoiceRow(stringResource(R.string.settings_two_herdr_both), stringResource(R.string.settings_two_herdr_both_sub, name), choice == 2) { choice = 2 }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SecondaryButton(stringResource(R.string.common_cancel), Modifier.weight(1f), onClick = onDismiss)
                PrimaryButton(stringResource(R.string.settings_save), Modifier.weight(1f)) {
                    onSave(
                        when (choice) {
                            0 -> listOf(SavedServer.BACKEND_TUI)
                            1 -> listOf(SavedServer.BACKEND_CORE)
                            else -> listOf(SavedServer.BACKEND_TUI, SavedServer.BACKEND_CORE)
                        },
                    )
                }
            }
        }
    }
}
