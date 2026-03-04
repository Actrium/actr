package com.example.actrdemo

import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import io.actorrtc.demo.R

class ServerActivity : AppCompatActivity() {

    private lateinit var statusText: TextView
    private lateinit var startButton: Button
    private lateinit var stopButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_server)

        // Initialize views
        statusText = findViewById(R.id.statusText)
        startButton = findViewById(R.id.startButton)
        stopButton = findViewById(R.id.stopButton)

        // Get signaling URL from intent
        val signalingUrl =
                intent.getStringExtra("signalingUrl") ?: "ws://10.0.2.2:8081/signaling/ws"

        // Set up button click listeners
        startButton.setOnClickListener {
            // TODO: Start server
            statusText.text = "Status: Starting..."
        }

        stopButton.setOnClickListener {
            // TODO: Stop server
            statusText.text = "Status: Stopping..."
        }

        // Load server fragment
        if (savedInstanceState == null) {
            // TODO: Create and load server fragment when available
        }
    }
}
