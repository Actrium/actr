package com.example.actrdemo

import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Captures all logcat output tagged "actr" (native library logs) and streams them
 * to the UI via the [onLines] callback. Uses a daemon thread + batched main-thread
 * delivery to avoid flooding the UI thread.
 *
 * On devices that restrict logcat access (e.g. Chinese OEMs), the logcat process
 * may exit immediately. The reader uses exponential backoff with a max retry limit
 * to avoid error spam.
 */
class LogcatReader(
    private val onLines: (String) -> Unit
) {
    private var thread: Thread? = null
    private var process: java.lang.Process? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private val buffer = StringBuilder()
    private val lock = Any()
    private var consecutiveFailures = 0

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
        if (thread?.isAlive == true) return

        thread = Thread {
            try {
                // Only capture "actr" tag, suppress GC/system noise.
                // Use -b main to explicitly target the main buffer (more compatible
                // with restricted devices than the default all-buffers mode).
                val pb = ProcessBuilder("logcat", "-b", "main", "-v", "threadtime", "actr:V", "*:S")
                // Do NOT redirect stderr to stdout — we read stderr separately to
                // diagnose why logcat failed (e.g. permission denied, logd not available).
                pb.redirectErrorStream(false)
                val proc = pb.start()
                process = proc

                // Drain stderr on a separate thread so logcat doesn't block
                val stderrThread = Thread({
                    try {
                        proc.errorStream.bufferedReader(Charsets.UTF_8).use { reader ->
                            var line = reader.readLine()
                            while (line != null) {
                                Log.w("LogcatReader", "logcat stderr: $line")
                                // Convert well-known logcat errors to user-friendly messages
                                val friendly = when {
                                    line.contains("Permission denied") ||
                                    line.contains("permission") ||
                                    line.contains("Operation not permitted") ->
                                        "[LogcatReader] logcat permission denied — device may restrict app log access"
                                    line.contains("Can't find") || line.contains("not found") ->
                                        "[LogcatReader] logcat binary unavailable on this device"
                                    line.contains("Unexpected EOF") || line.contains("unexpected EOF") ->
                                        null // suppress EOF noise, handled in stdout path
                                    else -> "[LogcatReader] logcat: $line"
                                }
                                if (friendly != null) {
                                    synchronized(lock) { buffer.append(friendly).append('\n') }
                                }
                                line = reader.readLine()
                            }
                        }
                    } catch (_: Exception) { /* stderr drain finished */ }
                }, "LogcatReader-stderr").apply { isDaemon = true }
                stderrThread.start()

                proc.inputStream.bufferedReader(Charsets.UTF_8).use { reader ->
                    // Successfully reading lines means logcat is working — reset failure count
                    consecutiveFailures = 0
                    while (!Thread.currentThread().isInterrupted) {
                        val line = reader.readLine()
                        if (line == null) {
                            val exitCode = try { proc.exitValue() } catch (_: Exception) { -1 }
                            Log.w("LogcatReader", "logcat exited with code=$exitCode")
                            break
                        }
                        synchronized(lock) { buffer.append(line).append('\n') }
                    }
                }
            } catch (_: InterruptedException) {
                // Normal shutdown
            } catch (e: Exception) {
                Log.e("LogcatReader", "logcat error", e)
            }

            // Exponential backoff retry with max attempts
            if (!Thread.currentThread().isInterrupted) {
                consecutiveFailures++
                val maxRetries = 5
                if (consecutiveFailures > maxRetries) {
                    val msg = "[LogcatReader] logcat unavailable after $maxRetries attempts — stopped retrying"
                    Log.w("LogcatReader", msg)
                    synchronized(lock) { buffer.append(msg).append('\n') }
                    // One last flush so the message reaches the UI
                    return@Thread
                }

                // 2s, 4s, 8s, 16s, 32s
                val delayMs = 2000L shl (consecutiveFailures - 1)
                val msg = "[LogcatReader] retrying in ${delayMs / 1000}s (attempt $consecutiveFailures/$maxRetries)..."
                Log.w("LogcatReader", msg)
                synchronized(lock) { buffer.append(msg).append('\n') }

                try { Thread.sleep(delayMs) } catch (_: InterruptedException) {}
                if (!Thread.currentThread().isInterrupted) {
                    thread = null
                    start()
                }
            }
        }.apply {
            name = "LogcatReader"
            isDaemon = true
        }

        consecutiveFailures = 0
        thread!!.start()
        mainHandler.post(flushRunnable)
    }

    fun stop() {
        mainHandler.removeCallbacks(flushRunnable)
        try { process?.destroy() } catch (_: Exception) {}
        process = null
        thread?.interrupt()
        thread = null
    }
}