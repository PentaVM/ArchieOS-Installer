package os.archie.capture

import android.content.Context
import android.content.SharedPreferences
import android.service.notification.StatusBarNotification
import org.json.JSONArray
import java.net.HttpURLConnection
import java.net.URL
import java.util.ArrayDeque
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Bounded, separate best-effort JSON transport. Never persists notification bodies. */
class WandaPoster(context: Context, deviceId: String, private val actor: ActorSession) {
    private val app = context.applicationContext
    private val collector = WandaCollector(deviceId)
    private val queue = ArrayDeque<WandaEvent>()
    private val executor = Executors.newSingleThreadScheduledExecutor()
    private val prefs = app.getSharedPreferences(WandaConfig.PREFS, Context.MODE_PRIVATE)
    private val actorPrefs = app.getSharedPreferences("archie_capture_actor", Context.MODE_PRIVATE)
    private val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> clear() }
    private var generation = 0L
    private var connection: HttpURLConnection? = null
    init {
        prefs.registerOnSharedPreferenceChangeListener(listener)
        actorPrefs.registerOnSharedPreferenceChangeListener(listener)
        executor.scheduleWithFixedDelay(::flush, 2, 2, TimeUnit.SECONDS)
    }
    @Synchronized fun accept(sbn: StatusBarNotification) {
        val config = WandaConfig.load(app)
        val currentActor = actor.current()
        // Check before reading text as well as before queueing it.
        if (!config.permits(currentActor)) { clear(); return }
        val snapshot = WandaNotification.read(sbn) ?: return
        for (event in collector.collect(snapshot, currentActor, config, sbn.postTime)) {
            if (queue.size >= 256) queue.removeFirst()
            queue.addLast(event)
        }
    }
    @Synchronized private fun clear() {
        queue.clear(); collector.clear(); generation++
        connection?.disconnect(); connection = null
    }
    private fun flush() {
        val config: WandaConfig
        val batch: JSONArray
        val batchGeneration: Long
        synchronized(this) {
            config = WandaConfig.load(app)
            if (!config.permits(actor.current())) { clear(); return }
            batchGeneration = generation
            batch = JSONArray()
            while (queue.isNotEmpty() && batch.length() < 32) {
                val event = queue.first()
                val trial = JSONArray(batch.toString()).put(event.toJson())
                if (trial.toString().toByteArray(Charsets.UTF_8).size > 256 * 1024) break
                batch.put(queue.removeFirst().toJson())
            }
            if (batch.length() == 0) return
        }
        var current: HttpURLConnection? = null
        try {
            synchronized(this) {
                if (batchGeneration != generation || config != WandaConfig.load(app) || !config.permits(actor.current())) return
                current = URL(config.endpoint).openConnection() as HttpURLConnection
                connection = current
            }
            val http = current ?: return
            http.requestMethod = "POST"
            http.instanceFollowRedirects = false
            http.connectTimeout = 4000; http.readTimeout = 4000; http.doOutput = true
            http.setRequestProperty("Content-Type", "application/json")
            http.setRequestProperty("Authorization", "Bearer ${config.token}")
            val bytes = batch.toString().toByteArray(Charsets.UTF_8)
            http.setFixedLengthStreamingMode(bytes.size)
            http.outputStream.use { it.write(bytes) }
            // Read no unbounded response body; counts stay on the server's status surface.
            http.responseCode
        } catch (_: Exception) { /* Dropped, no retry/disk spool or content logs. */ }
        finally { current?.disconnect(); synchronized(this) { if (connection === current) connection = null } }
    }
    fun close() {
        executor.shutdownNow(); clear()
        prefs.unregisterOnSharedPreferenceChangeListener(listener)
        actorPrefs.unregisterOnSharedPreferenceChangeListener(listener)
    }
}
