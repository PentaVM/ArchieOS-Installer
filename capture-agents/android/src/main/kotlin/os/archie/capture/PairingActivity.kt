package os.archie.capture

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity

/**
 * Where the owner pairs the phone to their node and confirms they are the owner.
 *
 * Two jobs: capture the node's address and pairing token (so the agent talks only to the
 * owner's node), and run the biometric confirmation that marks the session
 * [ActorSession.Actor.CONFIRMED_OWNER]. Both are the owner's deliberate acts; nothing here
 * runs without them.
 */
class PairingActivity : FragmentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val prefs = getSharedPreferences("archie_capture", Context.MODE_PRIVATE)
        val urlField = EditText(this).apply {
            hint = "https://your-node.local:port/capture"
            setText(prefs.getString("ingest_url", ""))
        }
        val tokenField = EditText(this).apply {
            hint = "pairing token from the node"
            setText(prefs.getString("token", ""))
        }
        val status = TextView(this)
        val wandaPrefs = getSharedPreferences(WandaConfig.PREFS, Context.MODE_PRIVATE)
        val wandaUrl = EditText(this).apply {
            hint = "Wanda: https://your-node/v1/events"
            setText(wandaPrefs.getString("endpoint", ""))
        }
        val wandaToken = EditText(this).apply {
            hint = "separate Wanda source token"
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
            setText(wandaPrefs.getString("token", ""))
        }
        val wandaEnabled = CheckBox(this).apply {
            text = "Enable WhatsApp notification content for Wanda"
            isChecked = wandaPrefs.getBoolean("enabled", false)
        }
        val wandaPresumed = CheckBox(this).apply {
            text = "Allow presumed-owner content after biometric confirmation decays"
            isChecked = wandaPrefs.getBoolean("allow_presumed", false)
        }
        val wandaSave = Button(this).apply {
            text = "Save Wanda pairing"
            setOnClickListener {
                val endpoint = wandaUrl.text.toString().trim()
                val token = wandaToken.text.toString().trim()
                if (!WandaConfig.validEndpoint(endpoint) || (wandaEnabled.isChecked && token.isEmpty())) {
                    status.text = "Use an HTTPS /v1/events URL and source token (HTTP only for loopback)."
                } else {
                    wandaPrefs.edit().putString("endpoint", endpoint).putString("token", token)
                        .putBoolean("enabled", wandaEnabled.isChecked).putBoolean("allow_presumed", wandaPresumed.isChecked).apply()
                    status.text = "Wanda pairing saved. Enroll device ${AgentConfig.load(applicationContext).deviceId} on your node. Notification previews only."
                }
            }
        }

        val save = Button(this).apply {
            text = "Pair this phone"
            setOnClickListener {
                prefs.edit()
                    .putString("ingest_url", urlField.text.toString().trim())
                    .putString("token", tokenField.text.toString().trim())
                    .apply()
                status.text = "Paired. Grant Notification access in Settings to begin."
            }
        }

        val confirm = Button(this).apply {
            text = "This is me (confirm owner)"
            setOnClickListener { confirmOwner(status) }
        }

        val notMe = Button(this).apply {
            text = "Someone else is using my phone"
            setOnClickListener {
                ActorSession(applicationContext).declareNotOwner()
                status.text = "Capture suppressed until you confirm you are back."
            }
        }

        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 96, 48, 48)
            addView(TextView(this@PairingActivity).apply {
                text = "Archie Capture records your own activity on this phone and sends it " +
                    "only to your node. When someone else uses the phone, it stops."
            })
            addView(urlField)
            addView(tokenField)
            addView(save)
            addView(TextView(this@PairingActivity).apply {
                text = "Wanda sends WhatsApp notification message content through a separate pairing. Hidden, disabled or redacted previews are unavailable. Domain is fixed by your node's source token."
            })
            addView(wandaUrl)
            addView(wandaToken)
            addView(wandaEnabled)
            addView(wandaPresumed)
            addView(wandaSave)
            addView(confirm)
            addView(notMe)
            addView(status)
        }.let { content -> android.widget.ScrollView(this).apply { addView(content) } })
    }

    private fun confirmOwner(status: TextView) {
        val executor = ContextCompat.getMainExecutor(this)
        val prompt = BiometricPrompt(this, executor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    ActorSession(applicationContext).confirmOwner()
                    status.text = "Confirmed. Your activity is attributed to you for the " +
                        "next while; it decays until you confirm again."
                }
            })
        prompt.authenticate(
            BiometricPrompt.PromptInfo.Builder()
                .setTitle("Confirm it is you")
                .setSubtitle("So Archie attributes this phone's activity to you")
                .setNegativeButtonText("Cancel")
                .build()
        )
    }
}
