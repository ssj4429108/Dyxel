package com.dyxel.android

import android.annotation.SuppressLint
import android.os.Bundle
import android.view.*
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.*

class MainActivity : AppCompatActivity() {

    private lateinit var engine: DyxelEngine
    private var isInitialized = false
    private var isInitializing = false
    private var choreographerCallback: Choreographer.FrameCallback? = null

    private fun startChoreographer() {
        choreographerCallback = object : Choreographer.FrameCallback {
            override fun doFrame(frameTimeNanos: Long) {
                DyxelEngine.nativeOnVBlank()
                Choreographer.getInstance().postFrameCallback(this)
            }
        }
        Choreographer.getInstance().postFrameCallback(choreographerCallback!!)
        android.util.Log.i("DyxelMain", "Choreographer VBlank callback started")
    }

    private fun stopChoreographer() {
        choreographerCallback?.let {
            Choreographer.getInstance().removeFrameCallback(it)
            choreographerCallback = null
            android.util.Log.i("DyxelMain", "Choreographer VBlank callback stopped")
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        engine = (application as MainApplication).dyxelEngine
        // Ensure engine preparation starts (extracts assets and calls prepareEngine)
        engine.setup(this)

        val sv = SurfaceView(this)
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
                    val totalStartTime = System.currentTimeMillis()
                    try {
                        val dataDir = filesDir.absolutePath
                        val wasmPath = "$dataDir/guest.wasm"
                        
                        // 1. Wait for engine core to be ready (WGPU instance, etc.)
                        val waitStartTime = System.currentTimeMillis()
                        var waitCount = 0
                        while (!engine.host.isEngineReady() && waitCount < 200) {
                            delay(50)
                            waitCount++
                        }
                        val waitElapsed = System.currentTimeMillis() - waitStartTime
                        android.util.Log.i("DyxelPerf", "[ColdStart] Wait for engine ready: ${waitElapsed}ms (waitCount=$waitCount)")
                        
                        if (!engine.host.isEngineReady()) {
                            android.util.Log.e("DyxelMain", "Engine core failed to become ready")
                            isInitializing = false
                            return@launch
                        }
                        
                        // 2. Initialize native surface (creates WGPU surface and starts render thread)
                        val initStartTime = System.currentTimeMillis()
                        val ptr = engine.getNativeSurface(holder.surface)
                        engine.host.initNative(ptr.toULong(), dataDir, width.toUInt(), height.toUInt())
                        val initElapsed = System.currentTimeMillis() - initStartTime
                        android.util.Log.i("DyxelPerf", "[ColdStart] Init native surface: ${initElapsed}ms")

                        // Start Choreographer VBlank callback after native surface is ready (must run on main thread)
                        runOnUiThread { startChoreographer() }

                        // 3. Load business logic
                        val wasmStartTime = System.currentTimeMillis()
                        engine.host.loadWasm(wasmPath)
                        val wasmElapsed = System.currentTimeMillis() - wasmStartTime
                        android.util.Log.i("DyxelPerf", "[ColdStart] Load WASM: ${wasmElapsed}ms")
                        
                        isInitialized = true
                        isInitializing = false
                        val totalElapsed = System.currentTimeMillis() - totalStartTime
                        android.util.Log.i("DyxelPerf", "[ColdStart] Total initialization: ${totalElapsed}ms")
                        android.util.Log.i("DyxelMain", "Dyxel initialized successfully")
                        
                    } catch (e: Exception) {
                        android.util.Log.e("DyxelMain", "Initialization failed", e)
                        isInitializing = false
                    }
                }
            }

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                isInitialized = false
                isInitializing = false
                stopChoreographer()
                // Synchronous barrier: block the UI thread until the render thread signals it has finished all GPU work.
                engine.host.stopNative()
            }
        })

        // Input Proxy: Multi-touch event handling
        sv.setOnTouchListener { v, event ->
            if (!isInitialized) return@setOnTouchListener false
            
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    engine.host.onPointerDown(
                        event.getPointerId(0).toUInt(),
                        event.x, 
                        event.y,
                        event.getPressure(0)
                    )
                }
                MotionEvent.ACTION_POINTER_DOWN -> {
                    val idx = event.actionIndex
                    engine.host.onPointerDown(
                        event.getPointerId(idx).toUInt(),
                        event.getX(idx),
                        event.getY(idx),
                        event.getPressure(idx)
                    )
                }
                MotionEvent.ACTION_MOVE -> {
                    // Batch process all pointer moves
                    for (i in 0 until event.pointerCount) {
                        engine.host.onPointerMove(
                            event.getPointerId(i).toUInt(),
                            event.getX(i),
                            event.getY(i)
                        )
                    }
                }
                MotionEvent.ACTION_UP -> {
                    engine.host.onPointerUp(
                        event.getPointerId(0).toUInt(),
                        event.x,
                        event.y
                    )
                }
                MotionEvent.ACTION_POINTER_UP -> {
                    val idx = event.actionIndex
                    engine.host.onPointerUp(
                        event.getPointerId(idx).toUInt(),
                        event.getX(idx),
                        event.getY(idx)
                    )
                }
                MotionEvent.ACTION_CANCEL -> {
                    engine.host.onPointerCancel()
                }
            }
            v.performClick()
            true
        }
    }
}
