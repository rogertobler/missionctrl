package ch.inscribe.missionctrl.ui

import android.app.Activity
import android.app.KeyguardManager
import android.content.Context
import android.content.ContextWrapper
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.KeyVault
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.KeyMaterial
import ch.inscribe.missionctrl.core.McException
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/** Fingerprint, face or device PIN – whatever the phone allows for Keystore keys – via Android's own prompt. */
object Unlock {
    /** A protected key needs a screen lock (PIN, pattern, password); fingerprint or face come on top. */
    fun possible(context: Context): Boolean = context.getSystemService(KeyguardManager::class.java)?.isDeviceSecure == true

    suspend fun ask(
        context: Context,
        title: String = context.getString(R.string.sshkeys_unlock_title),
        subtitle: String = context.getString(R.string.sshkeys_unlock_subtitle),
    ): Boolean {
        val activity = generateSequence(context) { (it as? ContextWrapper)?.baseContext }.filterIsInstance<Activity>().firstOrNull() ?: return false
        return suspendCancellableCoroutine { cont ->
            val signal = CancellationSignal()
            cont.invokeOnCancellation { signal.cancel() }
            val builder = BiometricPrompt.Builder(activity).setTitle(title).setSubtitle(subtitle)
            if (Build.VERSION.SDK_INT >= 30) {
                builder.setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL)
            } else {
                @Suppress("DEPRECATION") builder.setDeviceCredentialAllowed(true)
            }
            builder.build().authenticate(signal, activity.mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) { if (cont.isActive) cont.resume(true) }
                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) { if (cont.isActive) cont.resume(false) }
            })
        }
    }

    /** Runs [block]; if a protected key is locked, asks once and runs it again. Null when the person declined. */
    suspend fun <T> retrying(context: Context, core: Core, block: suspend () -> T): T? = try {
        block()
    } catch (e: Exception) {
        if (!KeyVault.needsUnlock(e)) throw e
        if (!ask(context)) null else { core.unlockKeys(); block() }
    }
}

/**
 * Servers wait for a locked key: each time the app is on the screen, first a try without a
 * question (the Keystore opens for 30 s after the phone was unlocked), then at most one question.
 * Only while resumed: asked behind the lock screen, Android drops the prompt at once.
 */
@Composable
fun UnlockOnStart(core: Core) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            var asked = false
            combine(core.lockedKeys, core.servers) { _, _ -> core.lockedInUse() }.collect { needed ->
                if (!needed || asked) return@collect
                core.unlockKeys()
                if (!core.lockedInUse()) return@collect
                asked = true
                delay(300) // let the window settle after coming to the front
                if (Unlock.ask(context)) core.unlockKeys()
            }
        }
    }
}

/** SSH keys: name and protection when made, rename, export with a passphrase. */
@Composable
fun KeysScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    val keys by core.keyList.collectAsState()
    val locked by core.lockedKeys.collectAsState()
    var renaming by remember { mutableStateOf<KeyVault.Entry?>(null) }
    var exporting by remember { mutableStateOf<KeyVault.Entry?>(null) }
    var confirmDelete by remember { mutableStateOf<KeyVault.Entry?>(null) }
    // The encrypted file between "Save file" and Android's save dialog.
    var pendingExport by remember { mutableStateOf<String?>(null) }
    fun toast(m: String?) = context.toast(m ?: resources.getString(R.string.sshkeys_error))
    val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        val text = pendingExport ?: return@rememberLauncherForActivityResult
        pendingExport = null
        if (uri != null) scope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    context.contentResolver.openOutputStream(uri)?.use { it.write(text.toByteArray()) } ?: error(resources.getString(R.string.sshkeys_write_failed))
                }
            }.onSuccess { toast(resources.getString(R.string.sshkeys_saved)) }.onFailure { toast(it.message) }
        }
    }
    KeyAdding(core) { adder ->
    Page(stringResource(R.string.sshkeys_title), nav) {
        if (keys.isEmpty()) {
            Block {
                Column(Modifier.padding(horizontal = 14.dp, vertical = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(stringResource(R.string.sshkeys_none_title), style = McType.bodyStrong, color = c.text)
                    Text(stringResource(R.string.sshkeys_none_text), style = McType.small, color = c.muted)
                }
            }
        }
        keys.forEach { k ->
            val renameDescription = stringResource(R.string.sshkeys_rename_desc, k.name)
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface)) {
                Column(Modifier.padding(start = 14.dp, top = 6.dp, end = 6.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        McIcon(McIcons.Key, c.text)
                        Row(
                            Modifier.weight(1f).heightIn(min = 44.dp).clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { renaming = k }
                                .semantics { contentDescription = renameDescription },
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            Text(k.name, style = McType.bodyStrong, color = c.text, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                            McIcon(McIcons.Pencil, c.muted, 14.dp)
                        }
                        Text(stringResource(if (k.origin == KeyVault.Origin.GENERATED) R.string.sshkeys_origin_generated else R.string.sshkeys_origin_imported), style = McType.caption, color = c.muted)
                        IconAction(McIcons.Close, stringResource(R.string.sshkeys_delete_desc, k.name), iconSize = 14.dp) { confirmDelete = k }
                    }
                    Text(k.algorithm + if (k.fingerprint.isNotEmpty()) " · " + k.fingerprint else "", style = McType.path, color = c.muted)
                    if (k.protected && k.id in locked) {
                        Row(
                            Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { scope.launch { if (Unlock.ask(context)) core.unlockKeys() } },
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            McIcon(McIcons.Lock, c.accentText, 13.dp)
                            Text(stringResource(R.string.sshkeys_locked_tap), style = McType.small, color = c.accentText)
                        }
                    } else {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            if (k.protected) McIcon(McIcons.Lock, c.accentText, 13.dp)
                            Text(stringResource(if (k.protected) R.string.sshkeys_protection_protected_line else R.string.sshkeys_protection_none_line), style = McType.small, color = c.muted)
                        }
                    }
                }
                ButtonBar(listOf(
                    BarButton("copy", stringResource(R.string.sshkeys_copy_public), McIcons.Copy, stringResource(R.string.sshkeys_copy)) {
                        Attachments.copy(context, "SSH public key", k.publicOpenssh); toast(resources.getString(R.string.sshkeys_public_copied))
                    },
                    BarButton("export", stringResource(R.string.sshkeys_export_named, k.name), McIcons.Download, stringResource(R.string.sshkeys_export)) { exporting = k },
                ))
            }
        }
        SectionTitle(stringResource(R.string.sshkeys_add))
        Block {
            AddKeyRows(adder)
            Divider()
            RowItem(stringResource(R.string.sshkeys_fido_title), stringResource(R.string.sshkeys_fido_subtitle), McIcons.SecurityKey)
        }
        Text(stringResource(R.string.sshkeys_storage_note), style = McType.caption, color = c.muted)
    }
    }

    renaming?.let { k -> RenameKeyDialog(k, onSave = { core.renameKey(k.id, it); renaming = null }, onDismiss = { renaming = null }) }
    exporting?.let { k ->
        ExportKeyDialog(core, k, onDismiss = { exporting = null }, onReady = { text ->
            exporting = null
            pendingExport = text
            save.launch(exportFileName(k.name))
        })
    }
    confirmDelete?.let { k ->
        ConfirmDialog(stringResource(R.string.sshkeys_delete_title, k.name), stringResource(R.string.sshkeys_delete_text), stringResource(R.string.sshkeys_delete),
            onConfirm = { confirmDelete = null; core.deleteKey(k.id) }, onDismiss = { confirmDelete = null })
    }
}

/** Opens the dialogs for a new or imported key; from the key page and the server form. */
internal class KeyAdder(val create: () -> Unit, val clipboard: () -> Unit, val file: () -> Unit)

/** Holds the create and import dialogs around [content], so they stay open while the page below changes. */
@Composable
internal fun KeyAdding(core: Core, onAdded: (KeyVault.Entry) -> Unit = {}, content: @Composable (KeyAdder) -> Unit) {
    val context = LocalContext.current
    val resources = LocalResources.current
    var creating by remember { mutableStateOf(false) }
    var importing by remember { mutableStateOf<Pair<String, String>?>(null) }
    val pick = rememberFilePicker { name, bytes -> importing = name to bytes.decodeToString() }
    val adder = remember(pick) {
        KeyAdder(
            create = { creating = true },
            clipboard = {
                when (val clip = Attachments.clipboard(context)) {
                    is ClipContent.Text -> importing = "" to clip.text
                    else -> context.toast(resources.getString(R.string.sshkeys_clipboard_no_text))
                }
            },
            file = pick,
        )
    }
    content(adder)
    if (creating) CreateKeyDialog(core, onDone = { creating = false }, onCreated = onAdded)
    importing?.let { (file, text) -> ImportKeyDialog(core, file, text, onDone = { importing = null }, onAdded = onAdded) }
}

/** Erzeugen, aus Zwischenablage, aus Datei – the same three rows on the key page and in the server form. */
@Composable
internal fun AddKeyRows(adder: KeyAdder) {
    RowItem(stringResource(R.string.sshkeys_create_title), stringResource(R.string.sshkeys_create_subtitle), McIcons.Plus, onClick = adder.create)
    Divider()
    RowItem(stringResource(R.string.sshkeys_import_clipboard_title), stringResource(R.string.sshkeys_import_clipboard_subtitle), McIcons.Paste, onClick = adder.clipboard)
    Divider()
    RowItem(stringResource(R.string.sshkeys_import_file_title), stringResource(R.string.sshkeys_import_file_subtitle), McIcons.File, onClick = adder.file)
}

/** "Galaxy Z Fold8" → id_ed25519_galaxy-z-fold8 */
internal fun exportFileName(name: String): String =
    "id_ed25519_" + name.lowercase().replace(Regex("[^a-z0-9]+"), "-").trim('-').ifEmpty { "missionctrl" }

/** "Biometrics or PIN" (recommended) / "None", with what each means; without a screen lock only "None". */
@Composable
private fun ProtectionChoice(protect: Boolean, onChange: (Boolean) -> Unit) {
    val c = LocalMc.current
    val context = LocalContext.current
    val possible = remember { Unlock.possible(context) }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(stringResource(R.string.sshkeys_protection_label), style = McType.small, color = c.muted)
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.bg)) {
            ChoiceRow(
                stringResource(R.string.sshkeys_protection_protected),
                stringResource(if (possible) R.string.sshkeys_protection_protected_subtitle else R.string.sshkeys_protection_needs_lock),
                protect, enabled = possible, badge = if (possible) stringResource(R.string.sshkeys_protection_recommended) else null,
            ) { onChange(true) }
            Divider()
            ChoiceRow(stringResource(R.string.sshkeys_protection_none), stringResource(R.string.sshkeys_protection_none_subtitle), !protect) { onChange(false) }
        }
        if (protect) Text(stringResource(R.string.sshkeys_protection_restart_note), style = McType.caption, color = c.muted)
    }
}

@Composable
private fun CreateKeyDialog(core: Core, onDone: () -> Unit, onCreated: (KeyVault.Entry) -> Unit = {}) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf(KeyVault.deviceName(context)) }
    var protect by remember { mutableStateOf(Unlock.possible(context)) }
    var busy by remember { mutableStateOf(false) }
    fun create() {
        val n = name.trim().ifEmpty { KeyVault.deviceName(context) }
        busy = true
        scope.launch {
            // A protected key is encrypted at once, so Android must have seen fingerprint, face or PIN just before.
            if (protect && !Unlock.ask(context, resources.getString(R.string.sshkeys_protect_title), resources.getString(R.string.sshkeys_protect_subtitle, n))) { busy = false; return@launch }
            runCatching { core.generateKey(n, protect) }
                .onSuccess { context.toast(resources.getString(R.string.sshkeys_created, n)); onCreated(it); onDone() }
                .onFailure { context.toast(it.message); busy = false }
        }
    }
    AlertDialog(
        onDismissRequest = onDone,
        containerColor = c.surface,
        title = { Text(stringResource(R.string.sshkeys_new_title), style = McType.title, color = c.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                LabeledField(stringResource(R.string.sshkeys_name), name, { name = it })
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(stringResource(R.string.sshkeys_authorized_keys_hint), style = McType.caption, color = c.muted)
                    Text("ssh-ed25519 AAAA… " + KeyVault.comment(name.ifBlank { KeyVault.deviceName(context) }), style = McType.path, color = c.text)
                }
                ProtectionChoice(protect) { protect = it }
            }
        },
        confirmButton = { ConfirmButton(stringResource(R.string.sshkeys_create), enabled = !busy, onClick = ::create) },
        dismissButton = { TextButton(onClick = onDone) { Text(stringResource(R.string.sshkeys_cancel), style = McType.body, color = c.muted) } },
    )
}

/** [file]: name of the picked file ("" from the clipboard). Asks for the file's passphrase only if it has one. */
@Composable
private fun ImportKeyDialog(core: Core, file: String, text: String, onDone: () -> Unit, onAdded: (KeyVault.Entry) -> Unit = {}) {
    val c = LocalMc.current
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()
    var material by remember { mutableStateOf<KeyMaterial?>(null) }
    var needsPass by remember { mutableStateOf(false) }
    var passphrase by remember { mutableStateOf("") }
    val defaultName = stringResource(R.string.sshkeys_imported_name)
    var name by remember { mutableStateOf(file.ifEmpty { defaultName }) }
    var protect by remember { mutableStateOf(Unlock.possible(context)) }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(text) {
        try {
            // A comment names the key better than a file name ("Fold 8 (MissionCtrl)" → "Fold 8").
            material = core.readKey(text, null).also { m -> if (m.comment.isNotBlank()) name = m.comment.removeSuffix(" (MissionCtrl)") }
        } catch (e: McException.PassphraseRequired) {
            needsPass = true
        } catch (e: McException) {
            context.toast(e.message); onDone()
        }
    }
    fun import() {
        busy = true
        scope.launch {
            try {
                val m = material ?: core.readKey(text, passphrase).also { material = it }
                val n = name.trim().ifEmpty { defaultName }
                if (protect && !Unlock.ask(context, resources.getString(R.string.sshkeys_protect_title), resources.getString(R.string.sshkeys_protect_subtitle, n))) { busy = false; return@launch }
                val entry = core.addKey(n, m, protect)
                Attachments.copy(context, "leer", "")
                context.toast(resources.getString(R.string.sshkeys_imported, n))
                onAdded(entry)
                onDone()
            } catch (e: McException.WrongPassphrase) {
                context.toast(resources.getString(R.string.sshkeys_wrong_passphrase)); busy = false
            } catch (e: Exception) {
                context.toast(e.message); busy = false
            }
        }
    }
    AlertDialog(
        onDismissRequest = onDone,
        containerColor = c.surface,
        title = { Text(stringResource(R.string.sshkeys_import_title), style = McType.title, color = c.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
                if (needsPass && material == null) LabeledField(stringResource(R.string.sshkeys_file_passphrase), passphrase, { passphrase = it }, password = true)
                LabeledField(stringResource(R.string.sshkeys_name), name, { name = it })
                ProtectionChoice(protect) { protect = it }
            }
        },
        confirmButton = {
            ConfirmButton(stringResource(R.string.sshkeys_import), enabled = !busy && (material != null || (needsPass && passphrase.isNotEmpty())), onClick = ::import)
        },
        dismissButton = { TextButton(onClick = onDone) { Text(stringResource(R.string.sshkeys_cancel), style = McType.body, color = c.muted) } },
    )
}

@Composable
private fun RenameKeyDialog(key: KeyVault.Entry, onSave: (String) -> Unit, onDismiss: () -> Unit) {
    val c = LocalMc.current
    var name by remember { mutableStateOf(key.name) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        title = { Text(stringResource(R.string.sshkeys_rename_title), style = McType.title, color = c.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                LabeledField(stringResource(R.string.sshkeys_name), name, { name = it })
                Text(stringResource(R.string.sshkeys_rename_note), style = McType.caption, color = c.muted)
            }
        },
        confirmButton = { TextButton(onClick = { onSave(name.trim()) }, enabled = name.isNotBlank()) { Text(stringResource(R.string.sshkeys_save), style = McType.body, color = c.text) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.sshkeys_cancel), style = McType.body, color = c.muted) } },
    )
}

/** Passphrase twice (at least 8), then [onReady] with the encrypted file; a protected key asks to unlock first. */
@Composable
private fun ExportKeyDialog(core: Core, key: KeyVault.Entry, onDismiss: () -> Unit, onReady: (String) -> Unit) {
    val c = LocalMc.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pass1 by remember { mutableStateOf("") }
    var pass2 by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val ready = pass1.length >= 8 && pass1 == pass2
    fun export() {
        busy = true
        scope.launch {
            runCatching { Unlock.retrying(context, core) { core.exportKey(key.id, pass1) } }
                .onSuccess { text -> if (text != null) onReady(text) else busy = false }
                .onFailure { context.toast(it.message); busy = false }
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        title = { Text(stringResource(R.string.sshkeys_export_named, key.name), style = McType.title, color = c.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(stringResource(R.string.sshkeys_export_note), style = McType.small, color = c.muted)
                LabeledField(stringResource(R.string.sshkeys_passphrase_min), pass1, { pass1 = it }, password = true)
                LabeledField(stringResource(R.string.sshkeys_passphrase_repeat), pass2, { pass2 = it }, password = true)
                when {
                    pass1.isNotEmpty() && pass1.length < 8 -> Text(stringResource(R.string.sshkeys_passphrase_too_short), style = McType.caption, color = c.accentText)
                    pass2.isNotEmpty() && pass1 != pass2 -> Text(stringResource(R.string.sshkeys_passphrase_mismatch), style = McType.caption, color = c.accentText)
                }
                if (key.protected) Text(stringResource(R.string.sshkeys_export_protected_note), style = McType.caption, color = c.muted)
                Text(stringResource(R.string.sshkeys_export_keep_apart), style = McType.caption, color = c.muted)
            }
        },
        confirmButton = { ConfirmButton(stringResource(R.string.sshkeys_save_file), enabled = ready && !busy, onClick = ::export) },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.sshkeys_cancel), style = McType.body, color = c.muted) } },
    )
}

/** A dialog's confirm action. Dimmed while it cannot be used, so it does not look ready. */
@Composable
private fun ConfirmButton(label: String, enabled: Boolean, onClick: () -> Unit) {
    val c = LocalMc.current
    TextButton(onClick = onClick, enabled = enabled) {
        Text(label, style = McType.body, color = if (enabled) c.text else c.text.copy(alpha = 0.38f))
    }
}
