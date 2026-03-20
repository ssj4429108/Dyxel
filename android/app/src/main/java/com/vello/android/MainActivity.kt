package com.vello.android

import android.annotation.SuppressLint
import android.os.Bundle
import android.view.*
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.*
import kotlin.coroutines.resume

class MainActivity : AppCompatActivity() {

    private lateinit var engine: VelloEngine
    private var surfaceView: SurfaceView? = null
    private var tickJob: Job? = null

    @SuppressLint("ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // 从 Application 获取引擎实例
        engine = (application as MainApplication).velloEngine

        val sv = SurfaceView(this)
        this.surfaceView = sv
        setContentView(sv, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT
        ))

        sv.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
            }

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val ptr = engine.getNativeSurface(holder.surface)

                // 使用协程异步初始化引擎，避免阻塞 UI 线程
                lifecycleScope.launch(Dispatchers.Default) {
                    // initNative 内部会自动检查是否需要 prepareEngine
                    // 由于已经在 Application 预热过，这里几乎是瞬间完成的
                    engine.host.initNative(ptr.toULong(), filesDir.absolutePath, width.toUInt(), height.toUInt())
                    startTick()
                }
            }


            override fun surfaceDestroyed(holder: SurfaceHolder) {
                stopTick()
                // 异步停止引擎
                lifecycleScope.launch(Dispatchers.Default) {
                    engine.host.stopNative()
                }
            }
        })

        sv.setOnTouchListener { v, event ->
            if (event.action == MotionEvent.ACTION_DOWN) {
                // 异步处理点击事件
                lifecycleScope.launch(Dispatchers.Default) {
                    engine.host.onTouch(event.x, event.y)
                }
                v.performClick()
            }
            true
        }
    }

    private fun startTick() {
        if (tickJob == null || tickJob?.isActive == false) {
            tickJob = lifecycleScope.launch(Dispatchers.Default) {
                android.util.Log.i("MainActivity", "Starting VSync-aligned render loop")
                while (isActive) {
                    try {
                        // 等待 VSync 信号对齐
                        awaitFrame()
                        if (engine.host.isInitialized()) {
                            engine.host.tick()
                        }
                    } catch (e: Exception) {
                        android.util.Log.e("MainActivity", "Tick error", e)
                    }
                }
                android.util.Log.i("MainActivity", "Stopping render loop")
            }
        }
    }

    /**
     * 利用 Choreographer 等待下一个硬件刷新信号
     */
    private suspend fun awaitFrame() = withContext(Dispatchers.Main) {
        suspendCancellableCoroutine<Long> { continuation ->
            val callback = Choreographer.FrameCallback { frameTimeNanos ->
                continuation.resume(frameTimeNanos)
            }
            Choreographer.getInstance().postFrameCallback(callback)
            continuation.invokeOnCancellation {
                Choreographer.getInstance().removeFrameCallback(callback)
            }
        }
    }

    private fun stopTick() {
        tickJob?.cancel()
        tickJob = null
    }
}
