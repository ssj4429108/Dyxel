package com.dyxel.android

import android.view.Surface
import uniffi.dyxel_core.DyxelHost
import kotlinx.coroutines.*

class DyxelEngine {
    val host: DyxelHost = DyxelHost()
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    fun setup(context: android.content.Context) {
        initLogger()

        scope.launch {
            // 1. Early start engine preparation (GPU/Vello initialization), don't wait for WASM extraction
            val dataDir = context.filesDir.absolutePath
            launch {
                try {
                    android.util.Log.i("DyxelEngine", "Starting engine preparation (parallel)")
                    host.prepareEngine(dataDir)
                } catch (e: Exception) {
                    android.util.Log.e("DyxelEngine", "Failed to prepare engine", e)
                }
            }

            // 2. Extract WASM assets in parallel
            launch {
                extractAssets(context)
            }
        }
    }



    private fun extractAssets(context: android.content.Context) {
        val destFile = java.io.File(context.filesDir, "guest.wasm")
        if (destFile.exists()) {
            android.util.Log.i("DyxelEngine", "guest.wasm already exists, skipping extraction")
            return
        }
        try {
            context.assets.open("guest.wasm").use { input ->
                java.io.FileOutputStream(destFile).use { output ->
                    input.copyTo(output)
                }
            }
            android.util.Log.i("DyxelEngine", "Successfully extracted guest.wasm")
        } catch (e: Exception) {
            android.util.Log.e("DyxelEngine", "Failed to extract guest.wasm", e)
        }
    }

    // JNI interface
    external fun getNativeSurface(surface: Surface): Long
    private external fun initLogger()

    companion object {
        init {
            System.setProperty("uniffi.component.dyxel_core.libraryOverride", "dyxel_core")
            System.loadLibrary("dyxel_core")
        }
    }
}
