package com.dyxel.android

import android.annotation.SuppressLint
import android.os.Bundle
import android.view.*
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.*

class MainActivity : AppCompatActivity() {

    private lateinit var engine: DyxelEngine
    private var surfaceView: SurfaceView? = null
    private var isInitialized = false
    private var isInitializing = false

    @SuppressLint("ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        engine = (application as MainApplication).dyxelEngine

        val sv = SurfaceView(this)
        this.surfaceView = sv
        setContentView(sv, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT
        ))

        sv.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {}

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                if (isInitialized) {
                    lifecycleScope.launch(Dispatchers.Default) {
                        engine.host.resizeNative(width.toUInt(), height.toUInt())
                    }
                    return
                }
                
                if (isInitializing) return
                isInitializing = true
                
                lifecycleScope.launch(Dispatchers.Default) {
                    try {
                        val dataDir = filesDir.absolutePath
                        val wasmPath = "$dataDir/guest.wasm"
                        
                        // Wait for engine ready
                        var waitCount = 0
                        while (!engine.host.isEngineReady() && waitCount < 200) {
                            delay(50)
                            waitCount++
                        }
                        if (!engine.host.isEngineReady()) {
                            throw IllegalStateException("Engine failed to become ready")
                        }
                        
                        val ptr = engine.getNativeSurface(holder.surface)
                        engine.host.initNative(ptr.toULong(), dataDir, width.toUInt(), height.toUInt())
                        
                        // Wait for surface initialized
                        waitCount = 0
                        while (!engine.host.isInitialized() && waitCount < 100) {
                            delay(50)
                            waitCount++
                        }
                        if (!engine.host.isInitialized()) {
                            throw IllegalStateException("Surface failed to initialize")
                        }
                        
                        engine.host.loadWasm(wasmPath)
                        isInitialized = true
                        
                    } catch (e: Exception) {
                        android.util.Log.e("DyxelMain", "Initialization failed", e)
                        isInitializing = false
                    }
                }
            }

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                isInitialized = false
                isInitializing = false
                lifecycleScope.launch(Dispatchers.Default) {
                    engine.host.stopNative()
                }
            }
        })

        sv.setOnTouchListener { v, event ->
            if (event.action == MotionEvent.ACTION_DOWN) {
                if (isInitialized) {
                    engine.host.onTouch(event.x, event.y)
                }
                v.performClick()
            }
            true
        }
    }
}
