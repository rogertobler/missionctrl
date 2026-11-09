package ch.inscribe.missionctrl

import android.content.Context
import android.os.Build
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.UserNotAuthenticatedException
import android.util.AtomicFile
import ch.inscribe.missionctrl.core.KeyMaterial
import ch.inscribe.missionctrl.core.generateClientKey
import java.io.File
import java.security.KeyStore
import java.util.UUID
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONArray
import org.json.JSONObject

/**
 * SSH client keys. Each private key is stored encrypted with an AES key that
 * never leaves the Android Keystore; metadata and ciphertext live in
 * noBackupFilesDir, which is never backed up. A [Entry.protected] key has its own
 * Keystore key that opens only for [UNLOCK_SECONDS] after fingerprint, face or PIN.
 */
class KeyVault(private val context: Context) {
    enum class Origin { GENERATED, IMPORTED }

    data class Entry(
        val id: String,
        val name: String,
        val publicOpenssh: String,
        val fingerprint: String,
        val algorithm: String,
        val origin: Origin,
        val protected: Boolean = false,
    )

    private val dir = context.noBackupFilesDir
    private val index = File(dir, "keys.json")

    // No key is made at first start; the server form asks for one when needed.
    init { migrateSingleKey() }

    fun list(): List<Entry> {
        // AtomicFile also finds a write that was cut off (backup file); absent → no keys yet.
        val raw = runCatching { AtomicFile(index).readFully() }.getOrNull() ?: return emptyList()
        val json = JSONArray(String(raw))
        return (0 until json.length()).map { json.getJSONObject(it) }.map {
            Entry(
                id = it.getString("id"),
                name = it.getString("name"),
                publicOpenssh = it.getString("public"),
                fingerprint = it.getString("fingerprint"),
                algorithm = it.getString("algorithm"),
                origin = Origin.valueOf(it.getString("origin")),
                protected = it.optBoolean("protected", false),
            )
        }
    }

    /** Throws [UserNotAuthenticatedException] for a protected key that is not unlocked ([needsUnlock]). */
    fun privateKey(id: String): String {
        val protected = list().firstOrNull { it.id == id }?.protected == true
        return decrypt(File(dir, "key_$id.enc").readBytes(), if (protected) protectedKey(id, create = false) else secretKey())
    }

    /** A [protect]ed key needs an unlock within [UNLOCK_SECONDS] before (the key is encrypted right away). */
    fun generate(name: String, protect: Boolean): Entry =
        store(name, generateClientKey(comment(name)), Origin.GENERATED, protect)

    fun import(name: String, material: KeyMaterial, protect: Boolean): Entry = store(name, material, Origin.IMPORTED, protect)

    /** A key made here also carries the new name in its public line (copied from now on). */
    fun rename(id: String, name: String) = change { list ->
        list.map {
            when {
                it.id != id -> it
                it.origin == Origin.GENERATED -> it.copy(name = name, publicOpenssh = withComment(it.publicOpenssh, comment(name)))
                else -> it.copy(name = name)
            }
        }
    }

    fun delete(id: String) {
        File(dir, "key_$id.enc").delete()
        runCatching { KeyStore.getInstance(KEYSTORE).apply { load(null) }.deleteEntry(protectedAlias(id)) }
        change { list -> list.filterNot { it.id == id } }
    }

    /** Read, change and write the index as one step: two at once (double tap) would lose an entry. */
    private fun change(edit: (List<Entry>) -> List<Entry>) = synchronized(this) { save(edit(list())) }

    // -- Server passwords (same protection as keys) -------------------------------

    fun savePassword(serverId: String, password: String) =
        write(File(dir, "pw_${safe(serverId)}.enc"), encrypt(password))

    fun password(serverId: String): String? =
        File(dir, "pw_${safe(serverId)}.enc").takeIf { it.exists() }?.let { decrypt(it.readBytes()) }

    fun deletePassword(serverId: String) {
        File(dir, "pw_${safe(serverId)}.enc").delete()
    }

    private fun safe(id: String) = id.filter { it.isLetterOrDigit() || it == '-' }

    private fun store(name: String, material: KeyMaterial, origin: Origin, protect: Boolean): Entry {
        val entry = Entry(
            id = UUID.randomUUID().toString(),
            name = name,
            publicOpenssh = material.publicOpenssh,
            fingerprint = material.fingerprint,
            algorithm = material.algorithm,
            origin = origin,
            protected = protect,
        )
        val key = if (protect) protectedKey(entry.id, create = true) else secretKey()
        try {
            write(File(dir, "key_${entry.id}.enc"), encrypt(material.privateOpenssh, key))
        } catch (e: Exception) {
            if (protect) runCatching { KeyStore.getInstance(KEYSTORE).apply { load(null) }.deleteEntry(protectedAlias(entry.id)) }
            throw e
        }
        change { it + entry }
        return entry
    }

    private fun save(entries: List<Entry>) {
        val json = JSONArray(
            entries.map {
                JSONObject()
                    .put("id", it.id).put("name", it.name).put("public", it.publicOpenssh)
                    .put("fingerprint", it.fingerprint).put("algorithm", it.algorithm)
                    .put("origin", it.origin.name).put("protected", it.protected)
            },
        )
        write(index, json.toString().toByteArray())
    }

    /** All or nothing: a crash while writing leaves the old file, never a broken one. */
    private fun write(file: File, data: ByteArray) {
        val atomic = AtomicFile(file)
        val out = atomic.startWrite()
        try { out.write(data); atomic.finishWrite(out) } catch (e: Exception) { atomic.failWrite(out); throw e }
    }

    /** The first build kept one key in client_key.enc/.pub; move it into the list. */
    private fun migrateSingleKey() {
        val oldPrivate = File(dir, "client_key.enc")
        val oldPublic = File(dir, "client_key.pub")
        if (!oldPrivate.exists() || !oldPublic.exists() || index.exists()) return
        val publicLine = oldPublic.readText().trim()
        val entry = Entry(
            id = DEVICE_KEY_ID,
            name = context.getString(R.string.keyvault_device_key_name),
            publicOpenssh = publicLine,
            fingerprint = "",
            algorithm = publicLine.substringBefore(' '),
            origin = Origin.GENERATED,
        )
        oldPrivate.copyTo(File(dir, "key_${entry.id}.enc"), overwrite = true)
        save(listOf(entry))
        oldPrivate.delete()
        oldPublic.delete()
    }

    private fun secretKey(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private fun protectedAlias(id: String) = "$ALIAS-$id"

    /** One Keystore key per protected SSH key; usable for [UNLOCK_SECONDS] after fingerprint, face or PIN. */
    private fun protectedKey(id: String, create: Boolean): SecretKey {
        val alias = protectedAlias(id)
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getEntry(alias, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        check(create) { context.getString(R.string.keyvault_protection_missing) }
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(true)
            .apply {
                if (Build.VERSION.SDK_INT >= 30) {
                    setUserAuthenticationParameters(UNLOCK_SECONDS, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
                } else {
                    @Suppress("DEPRECATION") setUserAuthenticationValidityDurationSeconds(UNLOCK_SECONDS)
                }
            }
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).apply { init(spec) }.generateKey()
    }

    /** Layout: 12-byte IV followed by the GCM ciphertext. */
    private fun encrypt(plain: String, key: SecretKey = secretKey()): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key)
        return cipher.iv + cipher.doFinal(plain.toByteArray())
    }

    private fun decrypt(data: ByteArray, key: SecretKey = secretKey()): String {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, data, 0, 12))
        return String(cipher.doFinal(data, 12, data.size - 12))
    }

    companion object {
        /** Id of the key created by the first build, kept so servers keep working. */
        const val DEVICE_KEY_ID = "device"
        /** How long a protected key opens after fingerprint, face or PIN; loaded once, it stays loaded. */
        const val UNLOCK_SECONDS = 30
        private const val KEYSTORE = "AndroidKeyStore"
        private const val ALIAS = "missionctrl-client-key"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"

        /** The phone's name as set in Android ("Galaxy Z Fold8"), else its model. */
        fun deviceName(context: Context): String =
            Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() } ?: Build.MODEL

        /** What servers show in authorized_keys: "Fold 8 (MissionCtrl)". */
        fun comment(name: String) = "${name.trim()} (MissionCtrl)"

        fun withComment(publicLine: String, comment: String): String =
            publicLine.trim().split(' ', limit = 3).take(2).joinToString(" ") + " " + comment

        /** True when [t] only means: unlock with fingerprint, face or PIN first. */
        fun needsUnlock(t: Throwable): Boolean = generateSequence(t) { it.cause }.any { it is UserNotAuthenticatedException }
    }
}
