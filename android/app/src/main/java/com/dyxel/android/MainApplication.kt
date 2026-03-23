package com.dyxel.android

import android.app.Application

class MainApplication : Application() {
    
    lateinit var dyxelEngine: DyxelEngine
        private set

    override fun onCreate() {
        super.onCreate()
        dyxelEngine = DyxelEngine()
        dyxelEngine.setup(this)
    }
}
