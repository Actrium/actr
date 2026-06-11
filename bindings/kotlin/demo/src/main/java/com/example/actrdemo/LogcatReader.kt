package com.example.actrdemo

import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Captures logcat output tagged "actr" (native library logs) and streams them
 * to the UI via the [onLines] callback.
 *
 * NOTE: On some devices (Chinese OEMs like vivo, OPPO, Xiaomi), the SELinux
 * policy blocks regular app processes from reading logcat — the logd connection
 * is refused immediately. When this is detected the reader gives up after a
 * few attempts to avoid error spam.
 *
 * Long-term solution: wire up the Rust-side LogCallback callback interface
 * (ffi/src/log_callback.rs) which forwards tracing events directly to Kotlin,
 * bypassing logcat entirely. This requires regenerating UniFFI bindings.
 */
class LogcatReader(
    private val onLines: (String) -> Unit
) {
    private var thread: Thread? = null
    private var process: java.lang.Process? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private val buffer = StringBuilder()
    private val lock = Any()

    // Retry state — accumulated across start() calls
    private var consecutiveFailures = 0
    private var gaveUp = false

    private val flushRunnable = object : Runnable {
        override fun run() {
            val batch = synchronized(lock) {
                if (buffer.isEmpty()) null
                else buffer.toString().also { buffer.clear() }
            }
            if (batch != null) onLines(batch)
            mainHandler.postDelayed(this, 100)
        }
    }

    fun start() {
        if (gaveUp || thread?.isAlive == true) return

        thread = Thread {
            try {
                val pb = ProcessBuilder("logcat", "-b", "main", "-v", "threadtime", "actr:V", "*:S")
                // Keep stderr separate — we only count stdout lines as "real" output
                pb.redirectErrorStream(false)
                val startMs = System.currentTimeMillis()
                val proc = pb.start()
                process = proc

                // Drain stderr silently on a background thread
                val stderrDone = java.util.concurrent.CountDownLatch(1)
                val stderrThread = Thread({
                    try {
                        proc.errorStream.bufferedReader(Charsets.UTF_8).use { it.readLine() }
                    } catch (_: Exception) {}
                    stderrDone.countDown()
                }, "LogcatReader-stderr").apply { isDaemon = true }
                stderrThread.start()

                var exitCode: Int
                var readAny = false
                proc.inputStream.bufferedReader(Charsets.UTF_8).use { reader ->
                    while (!Thread.currentThread().isInterrupted) {
                        val line = reader.readLine() ?: break
                        readAny = true
                        synchronized(lock) { buffer.append(line).append('\n') }
                    }
                    exitCode = try { proc.exitValue() } catch (_: Exception) { -1 }
                }

                // Wait briefly for stderr drain
                try { stderrDone.await(500, java.util.concurrent.TimeUnit.MILLISECONDS) } catch (_: Exception) {}

                val elapsed = System.currentTimeMillis() - startMs
                if (readAny) {
                    // Real output produced — logcat works
                    consecutiveFailures = 0
                    Log.d("LogcatReader", "logcat exited with code=$exitCode after ${elapsed}ms")
                    scheduleRetry(2000) // Normal restart delay
                } else {
                    // Zero stdout lines = logcat blocked on this device
                    consecutiveFailures++
                    Log.d(
                        "LogcatReader",
                        "logcat produced no output after ${elapsed}ms (attempt $consecutiveFailures/3)"
                    )

                    if (consecutiveFailures >= 3) {
                        val msg = "[LogcatReader] logcat blocked on this device — native logs unavailable. Consider wiring up LogCallback for native log capture."
                        Log.w("LogcatReader", msg)
                        synchronized(lock) { buffer.append(msg).append('\n') }
                        gaveUp = true
                        return@Thread
                    }

                    // Exponential backoff: 1s, 2s, 4s
                    scheduleRetry(1000L shl (consecutiveFailures - 1))
                }
            } catch (_: InterruptedException) {
                // Normal shutdown
            } catch (e: Exception) {
                Log.e("LogcatReader", "logcat error", e)
                consecutiveFailures++
                if (consecutiveFailures >= 3) {
                    gaveUp = true
                    return@Thread
                }
                scheduleRetry(2000L * consecutiveFailures)
            }
        }.apply {
            name = "LogcatReader"
            isDaemon = true
        }

        thread!!.start()
        mainHandler.post(flushRunnable)
    }

    private fun scheduleRetry(delayMs: Long) {
        if (Thread.currentThread().isInterrupted) return
        try { Thread.sleep(delayMs) } catch (_: InterruptedException) {}
        if (!Thread.currentThread().isInterrupted) {
            thread = null
            start()
        }
    }

    fun stop() {
        mainHandler.removeCallbacks(flushRunnable)
        try { process?.destroy() } catch (_: Exception) {}
        process = null
        thread?.interrupt()
        thread = null
    }
}