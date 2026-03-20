package com.vello.android

import android.annotation.SuppressLint
import android.os.Bundle
import android.view.*
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.*

class MainActivity : AppCompatActivity() {

    private lateinit var engine: VelloEngine
    private var surfaceView: SurfaceView? = null

    @SuppressLint("ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        engine = (application as MainApplication).velloEngine

        val sv = SurfaceView(this)
        this.surfaceView = sv
        setContentView(sv, ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT
        ))

        sv.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {}

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val ptr = engine.getNativeSurface(holder.surface)
                lifecycleScope.launch(Dispatchers.Default) {
                    if (engine.host.isInitialized()) {
                        engine.host.resizeNative(width.toUInt(), height.toUInt())
                    } else {
                        // 仅需调用 initNative，内部会自动 prepare 引擎并开启自主 Loop
                        engine.host.initNative(ptr.toULong(), filesDir.absolutePath, width.toUInt(), height.toUInt())
                    }
                }
            }

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                // 停止自主 Loop 并销毁 Surface
                engine.host.stopNative()
            }
        })

        sv.setOnTouchListener { v, event ->
            if (event.action == MotionEvent.ACTION_DOWN) {
                engine.host.onTouch(event.x, event.y)
                v.performClick()
            }
            true
        }
    }
}
