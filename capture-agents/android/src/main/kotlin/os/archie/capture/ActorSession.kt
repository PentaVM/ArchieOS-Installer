package os.archie.capture

import android.content.Context
import androidx.biometric.BiometricManager

/**
 * Who is at the device, tracked over time and shared across the app.
 *
 * This is the whole safety of capturing on a phone that gets handed around. Every event the
 * agent posts carries an [Actor], and the gate on the daemon suppresses anything that is not
 * the owner. The agent's job is to report that attribution honestly, and this class is where
 * it is decided.
 *
 * State lives in [android.content.SharedPreferences], not in the instance, so the confirmation
 * the owner makes in [PairingActivity] is seen by the [NotificationCaptureService] and
 * survives a process restart. Two objects, one truth.
 *
 * The model, and why it errs the way it does:
 *
 *  - A successful biometric confirmation marks the session [Actor.CONFIRMED_OWNER] for a
 *    window. This is the only positive signal that the owner is here.
 *  - After the window lapses without re-confirmation, the session decays to
 *    [Actor.PRESUMED_OWNER] — an unlocked personal phone is almost always its owner — and
 *    then, after longer, to [Actor.UNKNOWN].
 *  - The owner may declare "not me" (handing the phone over), which marks the session
 *    [Actor.CONFIRMED_OTHER] until they reclaim it. This is the "mum is using it" state.
 *
 * The agent never tries to work out *who* the other person is. Detecting that it is not the
 * owner is the entire feature; naming the guest would be the surveillance this is built to
 * avoid.
 */
class ActorSession(context: Context) {

    enum class Actor(val wire: String) {
        CONFIRMED_OWNER("confirmed_owner"),
        PRESUMED_OWNER("presumed_owner"),
        UNKNOWN("unknown"),
        CONFIRMED_OTHER("confirmed_other"),
    }

    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val biometrics = BiometricManager.from(context)

    /** Record a successful biometric confirmation that the owner is present. */
    fun confirmOwner() {
        prefs.edit()
            .putBoolean(KEY_OTHER, false)
            .putLong(KEY_CONFIRMED_AT, System.currentTimeMillis())
            .apply()
    }

    /** The owner is handing the phone to someone else. Suppresses capture until reclaimed. */
    fun declareNotOwner() {
        prefs.edit().putBoolean(KEY_OTHER, true).apply()
    }

    /** The owner reclaims the phone, e.g. after a fresh biometric confirmation. */
    fun reclaim() = confirmOwner()

    /**
     * The attribution to stamp on an event happening now.
     *
     * The decay from confirmed → presumed → unknown is deliberate: a phone left unlocked on a
     * table is progressively less certainly in the owner's hands, and the attribution should
     * weaken with time since the last positive signal rather than claim the owner forever.
     */
    fun current(now: Long = System.currentTimeMillis()): Actor {
        if (prefs.getBoolean(KEY_OTHER, false)) {
            return Actor.CONFIRMED_OTHER
        }
        // If the device cannot do biometrics at all, the strongest honest claim is
        // presumption — there is no signal that could ever confirm the owner, so never
        // claiming more than presumed is the truthful ceiling.
        val canConfirm =
            biometrics.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
                BiometricManager.BIOMETRIC_SUCCESS
        if (!canConfirm) {
            return Actor.PRESUMED_OWNER
        }
        val since = now - prefs.getLong(KEY_CONFIRMED_AT, 0L)
        return when {
            since <= CONFIRMED_WINDOW_MS -> Actor.CONFIRMED_OWNER
            since <= PRESUMED_WINDOW_MS -> Actor.PRESUMED_OWNER
            else -> Actor.UNKNOWN
        }
    }

    companion object {
        private const val PREFS = "archie_capture_actor"
        private const val KEY_OTHER = "explicit_other"
        private const val KEY_CONFIRMED_AT = "confirmed_at"

        // Fifteen minutes after a biometric confirmation, the owner is still confirmed.
        private const val CONFIRMED_WINDOW_MS = 15 * 60 * 1000L

        // For an hour after that, presumed. Beyond it, unknown — and the gate's
        // record_when_unknown default is false, so a phone untouched for over an hour and a
        // quarter captures nothing until the owner confirms again.
        private const val PRESUMED_WINDOW_MS = CONFIRMED_WINDOW_MS + 60 * 60 * 1000L
    }
}
