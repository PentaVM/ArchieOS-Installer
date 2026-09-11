package os.archie.capture

import android.content.Context
import java.util.UUID

/**
 * The agent's settings: which device this is, which node to send to, and the pairing token.
 *
 * The device id is generated once and kept, so a phone has one stable enrollment the owner
 * approves on the node. The ingest URL and token are set at pairing — the agent talks to the
 * owner's node and nowhere else.
 */
data class AgentConfig(
    val deviceId: String,
    val ingestUrl: String,
    val token: String,
) {
    companion object {
        private const val PREFS = "archie_capture"

        fun load(context: Context): AgentConfig {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            var deviceId = prefs.getString("device_id", null)
            if (deviceId == null) {
                // A stable, opaque id. Not a hardware serial — the owner approves *this
                // enrollment*, and a reinstall making a new id is a new enrollment they
                // approve again, which is the honest behaviour.
                deviceId = "phone-" + UUID.randomUUID().toString().take(12)
                prefs.edit().putString("device_id", deviceId).apply()
            }
            return AgentConfig(
                deviceId = deviceId,
                ingestUrl = prefs.getString("ingest_url", "") ?: "",
                token = prefs.getString("token", "") ?: "",
            )
        }
    }
}
