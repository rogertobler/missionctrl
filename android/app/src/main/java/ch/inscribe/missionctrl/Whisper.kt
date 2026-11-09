package ch.inscribe.missionctrl

import android.content.Context
import android.net.Uri
import android.util.Log
import androidx.annotation.StringRes
import ch.inscribe.missionctrl.core.SpeechModel
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** A whisper.cpp model as voxtype offers them (ggml files of ggerganov/whisper.cpp). */
data class WhisperModelInfo(
    val id: String,
    /** Technical name as whisper.cpp calls it ("large-v3-turbo"). */
    val name: String,
    val file: String,
    val bytes: Long,
    val sha256: String,
    /** Speed and accuracy on the phone, in the app's language. */
    @StringRes val noteRes: Int,
    /** Quantized file (q5): smaller and faster; its label says so. */
    val compressed: Boolean = false,
) {
    /** The name as the app shows it, "(compressed)" in the app's language. */
    fun label(context: Context): String = if (compressed) context.getString(R.string.whisper_label_compressed, name) else name
}

/**
 * Whisper models on the device: download on request, check, delete, load.
 *
 * The only network access besides SSH: on an explicit tap, the model file comes from
 * huggingface.co. No account, no identifiers; audio never leaves the phone.
 */
class WhisperStore(private val context: Context) {
    sealed interface Download {
        data class Running(val done: Long, val total: Long) : Download
        /** [res]: our own reason, shown in the app's language of the moment; else [detail] as it came (HTTP …). */
        data class Failed(@StringRes val res: Int?, val detail: String?) : Download
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val dir = File(context.filesDir, "whisper").apply { mkdirs() }
    private val jobs = mutableMapOf<String, Job>()

    /** Downloaded model ids. */
    val installed = MutableStateFlow(scan())
    val downloads = MutableStateFlow(emptyMap<String, Download>())

    private fun scan(): Set<String> = MODELS.filter { File(dir, it.file).isFile }.map { it.id }.toSet()

    fun file(id: String): File? = MODELS.firstOrNull { it.id == id }?.let { File(dir, it.file) }?.takeIf { it.isFile }

    fun download(model: WhisperModelInfo) {
        if (jobs[model.id]?.isActive == true) return
        val previous = jobs[model.id]
        jobs[model.id] = scope.launch {
            previous?.join() // a cancelled run must let go of the .part first
            val part = File(dir, model.file + ".part")
            try {
                // A complete .part (e.g. cut off before the check) is checked, not fetched again.
                if (part.length() >= model.bytes) {
                    if (part.length() == model.bytes && sha256(part) == model.sha256) { install(part, model); return@launch }
                    part.delete()
                }
                val digest = MessageDigest.getInstance("SHA-256")
                // Resume a broken download: hash what is there, then ask for the rest.
                var done = if (part.isFile) part.length() else 0L
                if (done > 0) part.inputStream().use { copyHashing(it, null, digest) }
                val conn = (URL(BASE_URL + model.file).openConnection() as HttpURLConnection).apply {
                    instanceFollowRedirects = true
                    connectTimeout = 15_000
                    readTimeout = 30_000
                    // Android's default names the device model and system version; the download needs neither.
                    setRequestProperty("User-Agent", "MissionCtrl")
                    if (done > 0) setRequestProperty("Range", "bytes=$done-")
                }
                try {
                    val code = conn.responseCode
                    if (code == 200 && done > 0) { done = 0; digest.reset() } // server ignored the range
                    if (code != 200 && code != 206) error("HTTP $code")
                    FileOutputStream(part, done > 0).use { out ->
                        conn.inputStream.use { input ->
                            copyHashing(input, out, digest) { n ->
                                ensureActive()
                                done += n
                                if (done % 2_000_000 < n) downloads.update { it + (model.id to Download.Running(done, model.bytes)) }
                            }
                        }
                    }
                } finally {
                    conn.disconnect()
                }
                if (hex(digest.digest()) != model.sha256) { part.delete(); throw Reason(R.string.whisper_error_checksum) }
                install(part, model)
            } catch (e: CancellationException) {
                downloads.update { it - model.id }
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "download ${model.id}: ${e.message}")
                val failed = if (e is Reason) Download.Failed(e.res, null) else Download.Failed(if (e.message == null) R.string.whisper_error_unknown else null, e.message)
                downloads.update { it + (model.id to failed) }
            }
        }
    }

    private fun install(part: File, model: WhisperModelInfo) {
        if (!part.renameTo(File(dir, model.file))) throw Reason(R.string.whisper_error_not_stored)
        downloads.update { it - model.id }
        installed.value = scan()
    }

    /** Stops a download; the part already fetched is kept for a later resume. */
    fun cancel(id: String) { jobs[id]?.cancel(); downloads.update { it - id } }

    fun delete(id: String) {
        cancel(id)
        scope.launch {
            jobs[id]?.join()
            MODELS.firstOrNull { it.id == id }?.let { File(dir, it.file).delete(); File(dir, it.file + ".part").delete() }
            lock.withLock { if (loadedId == id) unload() }
            installed.value = scan()
        }
    }

    /** Copies a ggml model the user picked (e.g. voxtype's) into the app; it must match one of [MODELS]. */
    suspend fun import(uri: Uri): String = withContext(Dispatchers.IO) {
        val tmp = File(dir, "import.part")
        try {
            val digest = MessageDigest.getInstance("SHA-256")
            val stream = context.contentResolver.openInputStream(uri) ?: error(context.getString(R.string.whisper_error_unreadable))
            stream.use { input -> FileOutputStream(tmp).use { out -> copyHashing(input, out, digest) } }
            val hash = hex(digest.digest())
            val model = MODELS.firstOrNull { it.sha256 == hash && it.bytes == tmp.length() } ?: error(context.getString(R.string.whisper_error_unknown_model))
            install(tmp, model)
            model.label(context)
        } finally {
            tmp.delete()
        }
    }

    private fun copyHashing(input: InputStream, out: OutputStream?, digest: MessageDigest, onChunk: (Int) -> Unit = {}) {
        val buf = ByteArray(1 shl 16)
        while (true) {
            val n = input.read(buf)
            if (n < 0) break
            out?.write(buf, 0, n)
            digest.update(buf, 0, n)
            onChunk(n)
        }
    }

    private fun sha256(file: File): String =
        MessageDigest.getInstance("SHA-256").also { d -> file.inputStream().use { copyHashing(it, null, d) } }.digest().let(::hex)

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

    private val lock = Mutex()
    private var loaded: SpeechModel? = null
    private var loadedId: String? = null

    /** The loaded model for [id], loading it on first use (seconds for large models). */
    suspend fun model(id: String): SpeechModel = lock.withLock {
        withContext(Dispatchers.IO) {
            loaded?.takeIf { loadedId == id }?.let { return@withContext it }
            unload()
            val f = file(id) ?: error(context.getString(R.string.whisper_error_not_downloaded))
            SpeechModel.load(f.path).also { loaded = it; loadedId = id }
        }
    }

    private fun unload() { loaded?.destroy(); loaded = null; loadedId = null }

    /** Frees the loaded model (app in the background, low memory); it loads again on the next take. */
    fun release() { scope.launch { lock.withLock { unload() } } }

    /** Starts loading in the background, so recognition is ready when speaking ends. */
    fun preload(id: String) { scope.launch { runCatching { model(id) } } }

    /** A failure of ours, as a string resource (its message for logs and callers that show it at once). */
    private inner class Reason(@StringRes val res: Int) : Exception(context.getString(res))

    companion object {
        private const val TAG = "Whisper"
        const val BASE_URL = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/"

        // Sizes and SHA-256 from huggingface.co/api/models/ggerganov/whisper.cpp (16.10.2026).
        // Times: 11 s of speech on a Galaxy Z Fold8 (6 threads, encoder window cut to the take).
        val MODELS = listOf(
            WhisperModelInfo("tiny", "tiny", "ggml-tiny.bin", 77_691_713, "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
                R.string.whisper_note_tiny),
            WhisperModelInfo("base", "base", "ggml-base.bin", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
                R.string.whisper_note_base),
            WhisperModelInfo("small", "small", "ggml-small.bin", 487_601_967, "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
                R.string.whisper_note_small),
            WhisperModelInfo("small-q5_1", "small", "ggml-small-q5_1.bin", 190_085_487, "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
                R.string.whisper_note_small_q5_1, compressed = true),
            WhisperModelInfo("medium-q5_0", "medium", "ggml-medium-q5_0.bin", 539_212_467, "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
                R.string.whisper_note_medium_q5_0, compressed = true),
            WhisperModelInfo("large-v3-turbo-q5_0", "large-v3-turbo", "ggml-large-v3-turbo-q5_0.bin", 574_041_195, "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
                R.string.whisper_note_large_v3_turbo_q5_0, compressed = true),
            WhisperModelInfo("large-v3-turbo", "large-v3-turbo", "ggml-large-v3-turbo.bin", 1_624_555_275, "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
                R.string.whisper_note_large_v3_turbo),
        )

        fun info(id: String) = MODELS.firstOrNull { it.id == id }
    }
}
