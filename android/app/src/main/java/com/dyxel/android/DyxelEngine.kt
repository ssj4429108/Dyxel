package com.dyxel.android

import android.view.Surface
import uniffi.dyxel_core.DyxelHost
import kotlinx.coroutines.*

class DyxelEngine {
    val host: DyxelHost = DyxelHost()
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private var enginePrepared = false

    fun setup(context: android.content.Context) {

        val dataDir = context.filesDir.absolutePath
        
        scope.launch {
            initLogger()
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
        val versionFile = java.io.File(context.filesDir, ".wasm_version")
        
        // Check if we need to update the WASM file
        // Use app update time as version marker
        val needsExtraction = try {
            val packageInfo = context.packageManager.getPackageInfo(context.packageName, 0)
            val lastUpdateTime = packageInfo.lastUpdateTime
            
            if (!destFile.exists() || !versionFile.exists()) {
                android.util.Log.i("DyxelEngine", "WASM file not found, extracting...")
                versionFile.writeText(lastUpdateTime.toString())
                true
            } else {
                val savedVersion = versionFile.readText().trim().toLongOrNull() ?: 0
                if (lastUpdateTime != savedVersion) {
                    android.util.Log.i("DyxelEngine", "App updated ($savedVersion -> $lastUpdateTime), re-extracting WASM...")
                    versionFile.writeText(lastUpdateTime.toString())
                    true
                } else {
                    android.util.Log.d("DyxelEngine", "WASM file up to date (version: $lastUpdateTime)")
                    false
                }
            }
        } catch (e: Exception) {
            android.util.Log.w("DyxelEngine", "Failed to check version, forcing re-extraction", e)
            true
        }
        
        if (!needsExtraction) {
            return
        }
        
        try {
            android.util.Log.i("DyxelEngine", "Extracting guest.wasm...")
            context.assets.open("guest.wasm").use { input ->
                java.io.FileOutputStream(destFile).use { output ->
                    input.copyTo(output)
                }
            }
            android.util.Log.i("DyxelEngine", "guest.wasm extracted successfully (${destFile.length()} bytes)")
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
