package os.archie.capture

import org.json.JSONObject
import java.security.MessageDigest

/** Separate content envelope. It never enters CaptureEvent or the metadata poster. */
data class WandaEvent(
    val eventId: String,
    val deviceId: String,
    val actor: String,
    val author: String,
    val chatId: String,
    val sender: String,
    val sourceMessageId: String? = null,
    val sentAtMs: Long?,
    val observedAtMs: Long,
    val text: String,
    val mediaOmitted: Boolean = false,
) {
    val coverage: String get() = "notification_preview"
    fun toJson(): JSONObject = JSONObject()
        .put("schema_version", 1).put("event_id", eventId).put("device_id", deviceId)
        .put("actor", actor).put("author", author).put("source", "android_notification")
        .put("chat_id", chatId).put("sender", sender)
        .put("source_message_id", sourceMessageId ?: JSONObject.NULL)
        .put("sent_at_ms", sentAtMs ?: JSONObject.NULL).put("observed_at_ms", observedAtMs)
        .put("text", text).put("media_omitted", mediaOmitted).put("coverage", coverage)

    companion object {
        internal fun digest(value: String): String = MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it.toInt() and 255) }
        internal fun validId(value: String): Boolean = value.isNotEmpty() && value.toByteArray(Charsets.UTF_8).size <= 192 &&
            value.none { it.code < 32 || it.code == 127 }
    }
}
