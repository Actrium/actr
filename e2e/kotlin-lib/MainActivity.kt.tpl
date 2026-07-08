package __PACKAGE__

import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity

/**
 * Minimal launcher activity for the e2e app.
 *
 * The actual actr echo round-trip is driven by the instrumentation test
 * (EchoIntegrationTest); this activity only needs to exist and compile so the
 * app APK builds and installs. (The fixture's MainActivity uses the pre-0.4
 * `createActrNode(configPath)` single-arg API which no longer compiles, so the
 * e2e replaces it with this stub.)
 */
class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
    }
}
