package com.example.actrdemo

import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import androidx.appcompat.app.AppCompatActivity
import io.actorrtc.demo.R

class MainActivity : AppCompatActivity() {

    private lateinit var signalingUrlInput: EditText
    private lateinit var serverButton: Button
    private lateinit var clientButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        setContentView(R.layout.activity_main)

        // Initialize views
        signalingUrlInput = findViewById(R.id.signalingUrlInput)
        serverButton = findViewById(R.id.serverButton)
        clientButton = findViewById(R.id.clientButton)

        // Set default signaling URL
        signalingUrlInput.setText("ws://10.0.2.2:8081/signaling/ws")

        // Set up button click listeners
        serverButton.setOnClickListener {
            val intent =
                    Intent(this, ServerActivity::class.java).apply {
                        putExtra("signalingUrl", signalingUrlInput.text.toString())
                    }
            startActivity(intent)
        }

        clientButton.setOnClickListener {
            val intent =
                    Intent(this, ClientActivity::class.java).apply {
                        putExtra("signalingUrl", signalingUrlInput.text.toString())
                    }
            startActivity(intent)
        }
    }
}
