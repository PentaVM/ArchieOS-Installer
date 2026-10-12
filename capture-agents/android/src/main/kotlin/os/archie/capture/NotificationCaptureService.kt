package os.archie.capture

import android.app.Notification
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification

/**
 * Reads the phone's live notification stream and turns it into capture events.
 *
 * `NotificationListenerService` is the supported Android API for this — the same one every
 * "read your notifications aloud" app uses. It delivers WhatsApp, Instagram, SMS and mail
 * message previews as they arrive, persistently, without a headless web session that would
 * violate a platform's terms and get an account banned. This is what "sesión abierta y
 * persistente" actually is on a phone: the OS's own live stream, not a scraped login.
 *
 * The original metadata channel remains body-free. An independently paired, disabled-by-
 * default Wanda channel can send notification previews through [WandaPoster], with the
 * actor gate checked before text is extracted. The rules below describe metadata only.
 *
 * What the metadata channel extracts, and what it refuses to:
 *
 *  - The *fact* of a message and *who* it is with (the counterparty), as metadata. That the
 *    owner is in contact with someone is what the graph keeps.
 *  - It does NOT forward the message text. The notification carries it; this service reads it
 *    only far enough to know a message happened and drops it. A received message is authored
 *    by the sender, and keeping the sender's words would attribute their private speech — the
 *    thing the daemon's gate refuses, refused here too so it never leaves the device.
 *  - It stamps every event with the current [ActorSession.Actor], so a message that arrives
 *    while someone other than the owner holds the phone is suppressed by the gate rather than
 *    folded into the owner's knowledge.
 */
class NotificationCaptureService : NotificationListenerService() {

    private lateinit var actor: ActorSession
    private lateinit var poster: EventPoster
    private lateinit var deviceId: String
    private lateinit var wanda: WandaPoster

    override fun onCreate() {
        super.onCreate()
        val config = AgentConfig.load(applicationContext)
        deviceId = config.deviceId
        actor = ActorSession(applicationContext)
        poster = EventPoster(config)
        wanda = WandaPoster(applicationContext, deviceId, actor)
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val packageName = sbn.packageName ?: return

        // App usage: which app produced a notification is a system fact, not anyone's private
        // speech, and it is the cheapest honest signal of what the owner is doing.
        emit(CaptureEvent.Source.APP_USAGE, CaptureEvent.Author.SYSTEM, counterparty = null)

        if (!isMessagingApp(packageName)) {
            return
        }

        // A message from a messaging app. The counterparty is the notification's title (the
        // sender or chat name). The body is deliberately NOT read into an event — only that a
        // message with this person happened.
        val extras = sbn.notification?.extras
        val counterparty = extras?.getCharSequence(Notification.EXTRA_TITLE)?.toString()

        // Whether the owner sent or received it is not always knowable from a notification;
        // an incoming message is the common case and the conservative one — treating it as
        // counterparty-authored means its content is metadata only, which is the safe default
        // even for the occasional self-notification.
        emit(
            CaptureEvent.Source.NOTIFICATION,
            CaptureEvent.Author.COUNTERPARTY,
            counterparty = counterparty,
        )
        wanda.accept(sbn)
    }

    override fun onDestroy() {
        wanda.close()
        super.onDestroy()
    }

    private fun emit(
        source: CaptureEvent.Source,
        author: CaptureEvent.Author,
        counterparty: String?,
    ) {
        poster.enqueue(
            CaptureEvent(
                deviceId = deviceId,
                source = source,
                actor = actor.current(),
                author = author,
                counterparty = counterparty,
            )
        )
    }

    private fun isMessagingApp(packageName: String): Boolean =
        packageName in MESSAGING_APPS

    companion object {
        // The apps whose notifications are message previews rather than system chatter. A
        // notification from any other app still counts as app-usage, but only these are read
        // for a counterparty.
        private val MESSAGING_APPS = setOf(
            "com.whatsapp",
            "com.whatsapp.w4b",
            "com.instagram.android",
            "com.facebook.orca", // Messenger
            "org.telegram.messenger",
            "com.google.android.apps.messaging", // SMS
            "com.google.android.gm", // Gmail
        )
    }
}
