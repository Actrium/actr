/**
 * Unified Handler Implementation
 *
 * This file provides the implementation for all local service handlers.
 * Implement your business logic in this class.
 */
package com.example

import android.util.Log
import com.example.generated.UnifiedHandler
import io.actor_rtc.actr.ContextBridge
import local_file.File.*


/**
 * Implementation of UnifiedHandler
 *
 * This class handles all local service requests.
 * Remote service requests are automatically forwarded by the UnifiedDispatcher.
 */
class MyUnifiedHandler : UnifiedHandler {

    companion object {
        private const val TAG = "MyUnifiedHandler"
    }

    // ===== LocalFileService methods =====
    // TODO: Implement your business logic for LocalFileService methods
    // Example method (adjust based on your actual proto definition):
    // override suspend fun your_method(request: YourRequest, ctx: ContextBridge): YourResponse {
    //     // Your implementation here
    // }

}
