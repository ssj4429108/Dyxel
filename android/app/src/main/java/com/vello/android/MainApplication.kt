package com.vello.android

import android.app.Application

class MainApplication : Application() {
    
    lateinit var velloEngine: VelloEngine
        private set

    override fun onCreate() {
        super.onCreate()
        velloEngine = VelloEngine()
        velloEngine.setup(this)
    }
}
