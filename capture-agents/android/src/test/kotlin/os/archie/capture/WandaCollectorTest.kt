package os.archie.capture

import android.app.Notification
import android.content.Context
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class WandaCollectorTest {
    private val paired = WandaConfig("https://owner.invalid/v1/events", "fixture-token", true, true)
    private fun notification(text: String = "Meet tomorrow", summary: Boolean = false): StatusBarNotification {
        val context: Context = RuntimeEnvironment.getApplication()
        val me = Person.Builder().setName("Me").build()
        val ana = Person.Builder().setName("Ana").build()
        val style = NotificationCompat.MessagingStyle(me).setConversationTitle("Family").setGroupConversation(true)
            .addMessage(text, 1790000000000, ana)
        val value = NotificationCompat.Builder(context, "fixture").setSmallIcon(android.R.drawable.ic_dialog_info)
            .setStyle(style).setGroupSummary(summary).build()
        return StatusBarNotification("com.whatsapp", "com.whatsapp", 1, "chat", 1000, 0, value, android.os.Process.myUserHandle(), null, 1790000000010)
    }
    @Test fun groupSenderIsSeparateFromConversationAndRepostDeduplicates() {
        val collector = WandaCollector("phone-fixture")
        val snapshot = WandaNotification.read(notification())!!
        val first = collector.collect(snapshot, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000020)
        assertEquals(1, first.size)
        assertEquals("Family", first.single().chatId)
        assertEquals("Ana", first.single().sender)
        assertEquals("counterparty", first.single().author)
        assertEquals(1790000000000, first.single().sentAtMs)
        assertEquals("notification_preview", first.single().coverage)
        assertTrue(collector.collect(snapshot, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000021).isEmpty())
        assertEquals(1, collector.collect(WandaNotification.read(notification("Edited plan"))!!, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000022).size)
        assertFalse(first.single().toJson().has("domain"))
    }
    @Test fun actorAndConfigurationSuppressBeforeDedupOrQueue() {
        val collector = WandaCollector("phone-fixture")
        val snapshot = WandaNotification.read(notification())!!
        for (actor in listOf(ActorSession.Actor.CONFIRMED_OTHER, ActorSession.Actor.UNKNOWN))
            assertTrue(collector.collect(snapshot, actor, paired, 1790000000020).isEmpty())
        assertTrue(collector.collect(snapshot, ActorSession.Actor.CONFIRMED_OWNER, paired.copy(enabled = false), 1790000000020).isEmpty())
        assertTrue(collector.collect(snapshot, ActorSession.Actor.PRESUMED_OWNER, paired.copy(allowPresumed = false), 1790000000020).isEmpty())
        assertEquals(1, collector.collect(snapshot, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000020).size)
        assertNull(WandaNotification.read(notification(summary = true)))
    }
    @Test fun fallbackTimeStaysUnknownAndLongMessagesAreRejected() {
        val context: Context = RuntimeEnvironment.getApplication()
        val value = NotificationCompat.Builder(context, "fixture").setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Ana").setContentText("Hello").build()
        val sbn = StatusBarNotification("com.whatsapp", "com.whatsapp", 1, "chat", 1000, 0, value, android.os.Process.myUserHandle(), null, 1790000000010)
        val collector = WandaCollector("phone-fixture")
        val event = collector.collect(WandaNotification.read(sbn)!!, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000020).single()
        assertNull(event.sentAtMs)
        assertTrue(event.toJson().isNull("sent_at_ms"))
        assertTrue(collector.collect(WandaNotification.read(notification("x".repeat(2001)))!!, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000020).isEmpty())
        assertTrue(collector.collect(WandaNotification.read(notification(""))!!, ActorSession.Actor.CONFIRMED_OWNER, paired, 1790000000020).isEmpty())
    }
}
