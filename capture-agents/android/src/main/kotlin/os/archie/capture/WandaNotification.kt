package os.archie.capture

import android.app.Notification
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat

data class WandaNotificationMessage(val sender: String, val text: String, val sentAtMs: Long?, val ownerAuthored: Boolean = false)
data class WandaNotificationSnapshot(val key: String, val chatId: String, val messages: List<WandaNotificationMessage>)

object WandaNotification {
    fun read(sbn: StatusBarNotification): WandaNotificationSnapshot? {
        if (sbn.packageName !in setOf("com.whatsapp", "com.whatsapp.w4b")) return null
        val notification = sbn.notification ?: return null
        if (notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        val extras = notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()?.trim().orEmpty()
        val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification)
        if (style != null) {
            val chat = style.conversationTitle?.toString()?.trim()?.takeIf { it.isNotEmpty() }
                ?: notification.shortcutId?.takeIf { it.isNotEmpty() } ?: title.ifEmpty { sbn.key }
            val messages = style.messages.takeLast(256).map { message ->
                val person = message.person
                WandaNotificationMessage(person?.name?.toString()?.trim()?.takeIf { it.isNotEmpty() }
                    ?: if (person == null) style.user.name?.toString()?.trim()?.takeIf { it.isNotEmpty() } ?: "self" else "unknown",
                    message.text?.toString().orEmpty(), message.timestamp.takeIf { it > 0 }, person == null)
            }
            return WandaNotificationSnapshot(sbn.key, chat, messages)
        }
        // A post time is an observation time, not a missing message's invented send time.
        val text = (extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
            ?: extras.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
        return WandaNotificationSnapshot(sbn.key, notification.shortcutId?.takeIf { it.isNotEmpty() }
            ?: title.ifEmpty { sbn.key }, listOf(WandaNotificationMessage(title.ifEmpty { "unknown" }, text, null)))
    }
}

class WandaCollector(private val deviceId: String) {
    // Only fingerprints are remembered. Notification bodies are never a durable spool.
    private val seen = LinkedHashSet<String>()
    @Synchronized fun clear() = seen.clear()
    @Synchronized fun collect(snapshot: WandaNotificationSnapshot, actor: ActorSession.Actor, config: WandaConfig, observedAtMs: Long): List<WandaEvent> {
        if (!config.permits(actor) || !WandaEvent.validId(deviceId) || !WandaEvent.validId(snapshot.chatId) || observedAtMs < 0) return emptyList()
        return snapshot.messages.take(256).mapNotNull { message ->
            val text = message.text.trim()
            if (text.isEmpty() || text.codePointCount(0, text.length) > 2000 || !WandaEvent.validId(message.sender)) return@mapNotNull null
            val author = if (message.ownerAuthored) "owner" else "counterparty"
            val material = org.json.JSONArray(listOf(deviceId, "android_notification", snapshot.key, snapshot.chatId,
                message.sender, author, message.sentAtMs ?: org.json.JSONObject.NULL, text)).toString()
            val id = WandaEvent.digest(material)
            if (!seen.add(id)) return@mapNotNull null
            if (seen.size > 2048) seen.iterator().run { next(); remove() }
            WandaEvent(id, deviceId, actor.wire, author, snapshot.chatId, message.sender,
                sentAtMs = message.sentAtMs, observedAtMs = observedAtMs, text = text)
        }
    }
}
