package os.archie.capture

/**
 * One observed thing, as posted to the Sentry capture ingest.
 *
 * The wire form is newline-delimited JSON matching `protocols/capture-event.schema.json`.
 * Deliberately holds no message body: a capture stream that carried the text of every message
 * would become the most damaging file on the phone. The graph keeps counts and contacts, and
 * this struct carries exactly what the graph needs and nothing it does not.
 */
data class CaptureEvent(
    val deviceId: String,
    val source: Source,
    val actor: ActorSession.Actor,
    val author: Author,
    /** When the content is a message, who the other party is as the device knows them. */
    val counterparty: String? = null,
    val observedAt: Long = System.currentTimeMillis(),
) {
    enum class Source(val wire: String) {
        TEXT_ENTRY("text_entry"),
        EMAIL("email"),
        WEB_SEARCH("web_search"),
        NAVIGATION("navigation"),
        APP_USAGE("app_usage"),
        NOTIFICATION("notification"),
        CLIPBOARD("clipboard"),
        FILE_ACTIVITY("file_activity"),
        DEVICE_LOCATION("device_location"),
    }

    enum class Author(val wire: String) {
        OWNER("owner"),
        COUNTERPARTY("counterparty"),
        SYSTEM("system"),
    }

    /** One line of NDJSON. Hand-built to avoid a JSON dependency and to guarantee no field
     *  the schema does not name — in particular, no message body — can leak in. */
    fun toNdjson(): String {
        val sb = StringBuilder()
        sb.append('{')
        appendString(sb, "device_id", deviceId)
        sb.append(',')
        appendString(sb, "source", source.wire)
        sb.append(',')
        appendString(sb, "actor", actor.wire)
        sb.append(',')
        appendString(sb, "author", author.wire)
        if (counterparty != null) {
            sb.append(',')
            appendString(sb, "counterparty", counterparty)
        }
        sb.append(',')
        appendString(sb, "observed_at", observedAt.toString())
        sb.append('}')
        return sb.toString()
    }

    private fun appendString(sb: StringBuilder, key: String, value: String) {
        sb.append('"').append(key).append("\":\"")
        for (c in value) {
            when (c) {
                '\\' -> sb.append("\\\\")
                '"' -> sb.append("\\\"")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> sb.append(c)
            }
        }
        sb.append('"')
    }
}
