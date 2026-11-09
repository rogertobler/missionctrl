package ch.inscribe.missionctrl.ui

import android.Manifest
import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.provider.OpenableColumns
import android.speech.RecognitionListener
import android.speech.RecognitionService
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.webkit.MimeTypeMap
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import ch.inscribe.missionctrl.CoreHolder
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.SpeechPrefs
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Speech input. Default is the on-device recognizer (no audio leaves the
 * phone); an installed recognition service can be chosen in the settings.
 * Results are post-processed like voxtype: replacements and ß → ss.
 */
/** [onResult] gets the text and whether the take is complete (false after a failure: never send it on its own). */
class VoiceInput(private val context: Context, private val prefs: () -> SpeechPrefs, private val onResult: (String, Boolean) -> Unit) {
    var listening by mutableStateOf(false)
        private set
    /** State for the bar under the field: "Listening… 0:12", "Recognizing…". */
    var partial by mutableStateOf("")
        private set
    /** Text recognized so far in this take, already post-processed; shown in the field while listening. */
    var live by mutableStateOf("")
        private set

    private var recognizer: SpeechRecognizer? = null
    private var recorder: WhisperRecorder? = null
    private var recognizing: Job? = null

    fun start() {
        val p = prefs()
        stop()
        if (p.engine == "whisper") { startWhisper(p); return }
        val service = p.engine.removePrefix("service:").takeIf { p.engine.startsWith("service:") }
        val r = if (service != null) {
            val component = ComponentName.unflattenFromString(service)
            if (component == null) { context.toast(context.getString(R.string.input_recognizer_not_found), long = true); return }
            SpeechRecognizer.createSpeechRecognizer(context, component)
        } else {
            // On-device recognition is Android 12+ (minSdk is 29).
            if (Build.VERSION.SDK_INT < 31 || !SpeechRecognizer.isOnDeviceRecognitionAvailable(context)) {
                context.toast(context.getString(R.string.input_no_on_device_recognizer), long = true)
                return
            }
            SpeechRecognizer.createOnDeviceSpeechRecognizer(context)
        }
        r.setRecognitionListener(object : RecognitionListener {
            override fun onReadyForSpeech(params: Bundle?) { listening = true }
            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rmsdB: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() {}
            override fun onError(error: Int) {
                listening = false
                if (error != SpeechRecognizer.ERROR_NO_MATCH && error != SpeechRecognizer.ERROR_SPEECH_TIMEOUT) {
                    context.toast(context.getString(R.string.input_recognizer_error, error))
                }
            }
            override fun onResults(results: Bundle?) {
                listening = false
                val raw = results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty()
                partial = ""; live = ""
                val text = SpeechText.process(raw, prefs())
                if (text.isNotBlank()) onResult(text, true)
            }
            override fun onPartialResults(partialResults: Bundle?) {
                live = SpeechText.process(partialResults?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty(), prefs())
            }
            override fun onEvent(eventType: Int, params: Bundle?) {}
        })
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            .putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, service == null)
        if (p.language.isNotBlank()) intent.putExtra(RecognizerIntent.EXTRA_LANGUAGE, p.language)
        // The pause that ends the take; recognizers may still stop on their own.
        val silence = (if (p.stopAfterMs > 0) p.stopAfterMs else WhisperRecorder.MAX_SECONDS * 1000).toLong()
        intent.putExtra(RecognizerIntent.EXTRA_SPEECH_INPUT_COMPLETE_SILENCE_LENGTH_MILLIS, silence)
        intent.putExtra(RecognizerIntent.EXTRA_SPEECH_INPUT_POSSIBLY_COMPLETE_SILENCE_LENGTH_MILLIS, silence)
        if (p.vocabulary.isNotEmpty() && Build.VERSION.SDK_INT >= 33) {
            intent.putStringArrayListExtra(RecognizerIntent.EXTRA_BIASING_STRINGS, ArrayList(p.vocabulary))
        }
        recognizer = r
        listening = true
        r.startListening(intent)
    }

    /** Stop listening and deliver what was heard. */
    fun finish() { recognizer?.stopListening(); recorder?.finish() }

    fun stop() {
        recognizer?.destroy()
        recognizer = null
        recorder?.cancel()
        recorder = null
        recognizing?.cancel()
        recognizing = null
        listening = false
        partial = ""
        live = ""
    }

    /**
     * Whisper on the device: record, then recognize. With [SpeechPrefs.whisperLive] each
     * sentence is recognized at the short pause after it while the speaker goes on, and shows in
     * [live]; at the end only the last part is left. Otherwise the whole take at once.
     */
    private fun startWhisper(p: SpeechPrefs) {
        val store = CoreHolder.get(context).whisper
        if (store.file(p.whisperModel) == null) {
            context.toast(context.getString(R.string.input_no_whisper_model), long = true)
            return
        }
        store.preload(p.whisperModel)
        val parts = Channel<ByteArray>(Channel.UNLIMITED)
        val heard = mutableListOf<String>()
        // Posted callbacks of an ended take must not touch the next one.
        lateinit var rec: WhisperRecorder
        rec = WhisperRecorder(
            split = p.whisperLive,
            silenceMs = p.stopAfterMs,
            onLevel = { seconds -> if (recorder === rec) partial = context.getString(R.string.input_listening_time, seconds / 60, seconds % 60) },
            onPart = { if (recorder === rec) parts.trySend(it) },
            onDone = { rest ->
                if (recorder === rec) {
                    partial = context.getString(R.string.input_recognizing)
                    parts.trySend(rest); parts.close()
                }
            },
        )
        if (!rec.start()) { context.toast(context.getString(R.string.input_mic_unavailable)); return }
        recorder = rec
        listening = true
        partial = context.getString(R.string.input_listening)
        recognizing = MainScope().launch {
            val result = runCatching {
                for (pcm in parts) {
                    if (pcm.isEmpty()) continue
                    val prompt = SpeechText.prompt(p.vocabulary, heard)
                    val started = System.currentTimeMillis()
                    val raw = withContext(Dispatchers.IO) { store.model(p.whisperModel).transcribe(pcm, p.language, prompt) }
                    Log.i("Whisper", "part %.1f s recognized in %.1f s".format(pcm.size / (WhisperRecorder.RATE * 2.0), (System.currentTimeMillis() - started) / 1000.0))
                    if (raw.isBlank()) continue
                    heard += raw
                    live = SpeechText.process(heard.joinToString(" "), prefs())
                }
            }
            // Cancelled meanwhile (✕ or a new take): drop the result.
            if (recorder !== rec) return@launch
            // A failed part ends the take: the microphone must not run on.
            rec.cancel()
            recorder = null; listening = false; partial = ""; live = ""
            result.onFailure { context.toast(context.getString(R.string.input_whisper_error, it.message), long = true) }
            // What was recognized before a failure still counts, as a draft.
            SpeechText.process(heard.joinToString(" "), prefs()).takeIf { it.isNotBlank() }?.let { onResult(it, result.isSuccess) }
        }
    }
}

/**
 * Records 16 kHz mono PCM for Whisper until [finish], or after a pause of [silenceMs] once
 * something was said (0: only [finish]); at most [MAX_SECONDS], with [split]
 * [MAX_SECONDS_SPLIT]. Without [split], [onDone] gets the whole take. With it, each stretch of
 * speech goes to [onPart] at the short pause after it ([endsPart]) and [onDone] gets only the rest; stretches without speech are dropped, since Whisper makes up
 * words for silence. Callbacks run on the main thread.
 */
class WhisperRecorder(
    private val split: Boolean,
    private val silenceMs: Int,
    private val onLevel: (Int) -> Unit,
    private val onPart: (ByteArray) -> Unit,
    private val onDone: (ByteArray) -> Unit,
) {
    @Volatile private var running = false
    @Volatile private var cancelled = false
    private var thread: Thread? = null
    private val main = Handler(Looper.getMainLooper())

    @SuppressLint("MissingPermission") // checked by rememberVoice
    fun start(): Boolean {
        val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (min <= 0) return false
        val record = runCatching {
            AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(min, RATE))
        }.getOrNull()?.takeIf { it.state == AudioRecord.STATE_INITIALIZED } ?: return false
        running = true
        thread = Thread {
            val out = ByteArrayOutputStream() // without split: the take
            val parts = PartSplitter()
            val buf = ByteArray(RATE / 5) // 100 ms
            var total = 0
            var spoke = false
            var quietMs = 0
            var lastSecond = -1
            try {
                record.startRecording()
                if (record.recordingState != AudioRecord.RECORDSTATE_RECORDING) running = false
                val maxBytes = RATE * 2 * (if (split) MAX_SECONDS_SPLIT else MAX_SECONDS)
                while (running && total < maxBytes) {
                    val n = record.read(buf, 0, buf.size)
                    if (n < 0) break // microphone gone (e.g. ERROR_DEAD_OBJECT): end the take
                    if (n == 0) continue
                    total += n
                    var sum = 0.0
                    for (i in 0 until n / 2) { val v = ((buf[2 * i + 1].toInt() shl 8) or (buf[2 * i].toInt() and 0xFF)).toShort().toDouble(); sum += v * v }
                    val speech = kotlin.math.sqrt(sum / (n / 2).coerceAtLeast(1)) > SPEECH_RMS
                    if (speech) { spoke = true; quietMs = 0 } else quietMs += n * 500 / RATE
                    if (split) parts.add(buf, n, speech)?.let { part -> if (!cancelled) main.post { onPart(part) } } else out.write(buf, 0, n)
                    if (silenceMs > 0 && spoke && quietMs >= silenceMs) break
                    val second = total / (RATE * 2)
                    if (second != lastSecond) { lastSecond = second; main.post { onLevel(second) } }
                }
            } finally {
                runCatching { record.stop() }; record.release()
            }
            val rest = if (split) parts.rest() else out.toByteArray()
            if (!cancelled) main.post { onDone(rest) }
        }.apply { name = "whisper-record"; start() }
        return true
    }

    fun finish() { running = false }
    fun cancel() { cancelled = true; running = false }

    companion object {
        const val RATE = 16_000
        const val MAX_SECONDS = 120
        /** Parts keep memory and recognition small, so a take may run longer (e.g. ended only with ✓). */
        const val MAX_SECONDS_SPLIT = 600
        const val SPEECH_RMS = 700.0

        /**
         * Whether a part of [bytes] with speech ends after [quietMs] of quiet: at a pause
         * between sentences, at a shorter one once the part is long, at the latest after
         * [PART_MAX_S] (Whisper takes at most 30 s at once).
         */
        fun endsPart(bytes: Int, quietMs: Int): Boolean {
            val seconds = bytes / (RATE * 2)
            return quietMs >= PART_PAUSE_MS || (seconds >= PART_LONG_S && quietMs >= SHORT_PAUSE_MS) || seconds >= PART_MAX_S
        }

        const val PART_PAUSE_MS = 700
        const val SHORT_PAUSE_MS = 250
        const val PART_LONG_S = 12
        const val PART_MAX_S = 25
    }
}

/**
 * Cuts a take into parts at pauses: speech plus the pause after it. Before speech only
 * [LEAD_IN_BYTES] stay, so a long silence (thinking, "only ✓") neither ends up in a part nor
 * makes the next one end at once and cut its first word.
 */
class PartSplitter {
    private val out = ByteArrayOutputStream()
    private var spoke = false
    private var quietMs = 0

    /** Adds [n] bytes of [buf]; returns a finished part, or null. */
    fun add(buf: ByteArray, n: Int, speech: Boolean): ByteArray? {
        out.write(buf, 0, n)
        if (speech) { spoke = true; quietMs = 0 } else quietMs += n * 500 / WhisperRecorder.RATE
        if (!spoke) {
            if (out.size() > LEAD_IN_BYTES) {
                val keep = out.toByteArray().copyOfRange(out.size() - LEAD_IN_BYTES, out.size())
                out.reset(); out.write(keep)
            }
            return null
        }
        if (!WhisperRecorder.endsPart(out.size(), quietMs)) return null
        return out.toByteArray().also { out.reset(); spoke = false }
    }

    /** What is left at the end, only if it holds speech. */
    fun rest(): ByteArray = if (spoke) out.toByteArray() else ByteArray(0)

    companion object {
        /** Half a second before speech: the threshold notices a word a little late. */
        const val LEAD_IN_BYTES = WhisperRecorder.RATE
    }
}

/** Remembers a [VoiceInput], asking for the microphone permission on first use. */
@Composable
fun rememberVoice(prefs: SpeechPrefs, onResult: (heard: String, complete: Boolean) -> Unit): Pair<VoiceInput, () -> Unit> {
    val context = LocalContext.current
    val current = rememberUpdatedState(prefs)
    val callback = rememberUpdatedState(onResult)
    val voice = remember { VoiceInput(context, { current.value }) { heard, complete -> callback.value(heard, complete) } }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) voice.start()
    }
    DisposableEffect(voice) { onDispose { voice.stop() } }
    val start = {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            voice.start()
        } else {
            permission.launch(Manifest.permission.RECORD_AUDIO)
        }
    }
    return voice to start
}

/** Post-processing of recognized text, like voxtype. */
object SpeechText {
    fun process(text: String, p: SpeechPrefs): String {
        var out = text
        for ((from, to) in p.replacements) {
            if (from.isBlank()) continue
            out = Regex("(?i)(?<![\\p{L}\\p{N}])" + Regex.escape(from) + "(?![\\p{L}\\p{N}])").replace(out, Regex.escapeReplacement(to))
        }
        if (p.swissSpelling) out = out.replace("ß", "ss").replace("ẞ", "SS")
        return out
    }

    /**
     * Whisper's prompt: the words to expect (like voxtype's `initial_prompt`), then the end of
     * what was heard so far as context for the next part. Whisper keeps the end of a long
     * prompt, so the context is cut to [CONTEXT_CHARS].
     */
    fun prompt(vocabulary: List<String>, heard: List<String>): String =
        listOf(vocabulary.joinToString(", "), heard.joinToString(" ").takeLast(CONTEXT_CHARS).trimStart())
            .filter { it.isNotBlank() }.joinToString(" ")

    private const val CONTEXT_CHARS = 200

    /** Read `initial_prompt` and `replacements = { "a" = "b", … }` from a voxtype config. */
    fun fromVoxtype(toml: String): Pair<List<String>, List<Pair<String, String>>> {
        // voxtype's default config carries a commented-out `# replacements = { … }` example.
        val config = toml.lines().filterNot { it.trimStart().startsWith("#") }.joinToString("\n")
        val prompt = Regex("""initial_prompt\s*=\s*"([^"]*)"""").find(config)?.groupValues?.get(1).orEmpty()
        val words = prompt.split(',').map { it.trim().trimEnd('.') }.filter { it.isNotEmpty() }
        val block = Regex("""replacements\s*=\s*\{([^}]*)\}""").find(config)?.groupValues?.get(1).orEmpty()
        val pairs = Regex(""""([^"]+)"\s*=\s*"([^"]*)"""").findAll(block).map { it.groupValues[1] to it.groupValues[2] }.toList()
        return words to pairs
    }

    /** Installed recognition services: (flattened component, label). */
    fun services(context: Context): List<Pair<String, String>> {
        val pm = context.packageManager
        return pm.queryIntentServices(Intent(RecognitionService.SERVICE_INTERFACE), 0).map {
            val info = it.serviceInfo
            ComponentName(info.packageName, info.name).flattenToString() to it.loadLabel(pm).toString()
        }.distinctBy { it.first }.filterNot { SpeechPrefs.unusable("service:${it.first}") }
    }
}

/** What the clipboard holds. */
sealed interface ClipContent {
    data class Text(val text: String) : ClipContent
    data class File(val uri: Uri, val mime: String?) : ClipContent
    data object Empty : ClipContent
}

object Attachments {
    fun clipboard(context: Context): ClipContent {
        val cm = context.getSystemService(ClipboardManager::class.java) ?: return ClipContent.Empty
        val clip = cm.primaryClip ?: return ClipContent.Empty
        if (clip.itemCount == 0) return ClipContent.Empty
        val item = clip.getItemAt(0)
        item.uri?.let { uri ->
            val mime = context.contentResolver.getType(uri) ?: clip.description.getMimeType(0)
            return ClipContent.File(uri, mime)
        }
        val text = item.coerceToText(context)?.toString().orEmpty()
        return if (text.isEmpty()) ClipContent.Empty else ClipContent.Text(text)
    }

    fun copy(context: Context, label: String, text: String) {
        context.getSystemService(ClipboardManager::class.java)
            ?.setPrimaryClip(ClipData.newPlainText(label, text))
    }

    /** Largest file the app uploads (it is held in memory once). */
    const val MAX_UPLOAD_BYTES = 25L * 1024 * 1024

    /** Bytes and a file name for an uri (gallery, files, clipboard image); on [Dispatchers.IO]. */
    suspend fun read(context: Context, uri: Uri, mime: String?): Pair<String, ByteArray> = withContext(Dispatchers.IO) {
        var name: String? = null
        var size = -1L
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) { name = c.getString(0); if (!c.isNull(1)) size = c.getLong(1) }
        }
        require(size <= MAX_UPLOAD_BYTES) { context.getString(R.string.input_file_too_large) }
        val ext = mime?.let { MimeTypeMap.getSingleton().getExtensionFromMimeType(it) }?.let { ".$it" }.orEmpty()
        // The size column can be missing; the cap holds while reading too.
        val bytes = context.contentResolver.openInputStream(uri)?.use { input ->
            val out = ByteArrayOutputStream()
            val buf = ByteArray(1 shl 16)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                require(out.size() <= MAX_UPLOAD_BYTES) { context.getString(R.string.input_file_too_large) }
            }
            out.toByteArray()
        } ?: ByteArray(0)
        (name ?: "upload$ext") to bytes
    }
}

/** File picker that returns (name, bytes), read off the main thread. */
@Composable
fun rememberFilePicker(onPicked: (String, ByteArray) -> Unit): () -> Unit {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) scope.launch {
            runCatching { Attachments.read(context, uri, context.contentResolver.getType(uri)) }
                .onSuccess { (name, bytes) -> onPicked(name, bytes) }
                .onFailure { context.toast(it.message, long = true) }
        }
    }
    return { launcher.launch(arrayOf("*/*")) }
}
