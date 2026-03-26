package com.dyxel.android

import android.view.Surface
import uniffi.dyxel_core.DyxelHost
import kotlinx.coroutines.*

class DyxelEngine {
    val host: DyxelHost = DyxelHost()
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private var enginePrepared = false

    fun setup(context: android.content.Context) {
        initLogger()
        
        val dataDir = context.filesDir.absolutePath
        
        scope.launch {
            launch { extractAssets(context) }
            launch {
                if (!enginePrepared) {
                    try {
                        host.prepareEngine(dataDir)
                        enginePrepared = true
                    } catch (e: Exception) {
                        android.util.Log.e("DyxelEngine", "Failed to prepare engine", e)
                    }
                }
            }
        }
    }

    private fun extractAssets(context: android.content.Context) {
        val destFile = java.io.File(context.filesDir, "guest.wasm")
        if (destFile.exists()) return
        try {
            context.assets.open("guest.wasm").use { input ->
                java.io.FileOutputStream(destFile).use { output ->
                    input.copyTo(output)
                }
            }
        } catch (e: Exception) {
            android.util.Log.e("DyxelEngine", "Failed to extract guest.wasm", e)
        }
    }

    external fun getNativeSurface(surface: Surface): Long
    private external fun initLogger()

    companion object {
        init {
            System.setProperty("uniffi.component.dyxel_core.libraryOverride", "dyxel_core")
            System.loadLibrary("dyxel_core")
        }
    }
}
