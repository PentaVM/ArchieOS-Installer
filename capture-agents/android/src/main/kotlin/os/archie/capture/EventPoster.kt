package os.archie.capture

import java.io.OutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * Batches capture events and posts them to the owner's node as newline-delimited JSON.
 *
 * The node is the owner's own — a LAN address or a paired tunnel, configured once. Events go
 * only there. This is the transport half of "linkearse solo a mí": the agent does not fan out
 * to any cloud, it delivers to the one node the owner runs.
 *
 * Batched and best-effort. Losing a few app-usage events to a dropped connection costs a
 * count, not correctness, so the poster does not persist a durable queue — it holds a bounded
 * in-memory buffer, flushes on a timer, and drops the oldest if the node is unreachable for
 * long enough. The alternative, a durable on-disk spool of everything the owner did, is
 * exactly the accumulation the whole design avoids.
 */
class EventPoster(private val config: AgentConfig) {

    private val queue = ConcurrentLinkedQueue<CaptureEvent>()
    private val flusher = Executors.newSingleThreadScheduledExecutor()

    init {
        flusher.scheduleWithFixedDelay(::flush, FLUSH_SECONDS, FLUSH_SECONDS, TimeUnit.SECONDS)
    }

    fun enqueue(event: CaptureEvent) {
        queue.add(event)
        // Bound the buffer. Dropping the oldest keeps the newest — a message that just
        // arrived matters more than one from ten minutes ago — and either way nothing is
        // spooled to disk.
        while (queue.size > MAX_BUFFERED) {
            queue.poll()
        }
    }

    private fun flush() {
        if (queue.isEmpty()) {
            return
        }
        val batch = buildString {
            var drained = 0
            while (drained < MAX_BATCH) {
                val event = queue.poll() ?: break
                append(event.toNdjson()).append('\n')
                drained++
            }
        }
        if (batch.isEmpty()) {
            return
        }
        try {
            post(batch)
        } catch (_: Exception) {
            // Best-effort. A failed flush drops this batch rather than retrying forever; the
            // node's picture of what the owner did has a small hole, which is survivable, and
            // no queue grows without bound on the phone.
        }
    }

    private fun post(ndjson: String) {
        val connection = URL(config.ingestUrl).openConnection() as HttpURLConnection
        connection.requestMethod = "POST"
        connection.connectTimeout = 4000
        connection.readTimeout = 4000
        connection.doOutput = true
        connection.setRequestProperty("Content-Type", "application/x-ndjson")
        // The node authenticates the agent with a token the owner set at pairing. It is a
        // bearer of "this is the owner's enrolled device", nothing about the content.
        connection.setRequestProperty("Authorization", "Bearer ${config.token}")
        connection.outputStream.use { out: OutputStream ->
            out.write(ndjson.toByteArray(Charsets.UTF_8))
        }
        connection.inputStream.use { it.readBytes() }
        connection.disconnect()
    }

    companion object {
        private const val FLUSH_SECONDS = 10L
        private const val MAX_BUFFERED = 2000
        private const val MAX_BATCH = 500
    }
}
