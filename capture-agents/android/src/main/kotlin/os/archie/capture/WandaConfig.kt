package os.archie.capture

import android.content.Context
import java.net.URI

/** This pairing is independent of the Sentry metadata URL/token. Domain is server-owned. */
data class WandaConfig(val endpoint: String, val token: String, val enabled: Boolean, val allowPresumed: Boolean) {
    fun permits(actor: ActorSession.Actor): Boolean = enabled && token.isNotEmpty() && validEndpoint(endpoint) &&
        (actor == ActorSession.Actor.CONFIRMED_OWNER || (actor == ActorSession.Actor.PRESUMED_OWNER && allowPresumed))
    companion object {
        const val PREFS = "archie_wanda"
        fun load(context: Context): WandaConfig {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            return WandaConfig(prefs.getString("endpoint", "") ?: "", prefs.getString("token", "") ?: "",
                prefs.getBoolean("enabled", false), prefs.getBoolean("allow_presumed", false))
        }
        fun validEndpoint(value: String): Boolean = try {
            val uri = URI(value)
            uri.userInfo == null && uri.path == "/v1/events" && uri.query == null && uri.fragment == null && uri.host != null &&
                (uri.scheme == "https" || (uri.scheme == "http" && uri.host in setOf("localhost", "127.0.0.1", "[::1]")))
        } catch (_: Exception) { false }
    }
}
