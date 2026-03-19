package com.vello.android

import android.view.Surface
import uniffi.host_core.VelloHost
import kotlinx.coroutines.*

class VelloEngine {
    val host: VelloHost = VelloHost()
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    fun setup(context: android.content.Context) {
        initLogger()
        // 1. 立即同步解压资产，确保文件存在
        extractAssets(context)
        
        // 2. 异步预热引擎
        scope.launch {
            try {
                host.prepareEngine(context.filesDir.absolutePath)
            } catch (e: Exception) {
                android.util.Log.e("VelloEngine", "Failed to prepare engine", e)
            }
        }
    }

    private fun extractAssets(context: android.content.Context) {
        val destFile = java.io.File(context.filesDir, "guest.wasm")
        try {
            context.assets.open("guest.wasm").use { input ->
                java.io.FileOutputStream(destFile).use { output ->
                    input.copyTo(output)
                }
            }
            android.util.Log.i("VelloEngine", "Successfully updated guest.wasm")
        } catch (e: Exception) {
            android.util.Log.e("VelloEngine", "Failed to extract guest.wasm", e)
        }
    }

    // JNI 接口
    external fun getNativeSurface(surface: Surface): Long
    private external fun initLogger()

    companion object {
        init {
            System.setProperty("uniffi.component.host_core.libraryOverride", "host_android")
            System.loadLibrary("host_android")
        }
    }
}
