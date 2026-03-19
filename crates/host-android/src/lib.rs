extern crate host_core;
use jni::sys::{jlong, jobject, JNIEnv};

// 导出 host_core 的 UniFFI 绑定
host_core::uniffi_reexport_scaffolding!();

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn Java_com_vello_android_VelloEngine_getNativeSurface(
    env: *mut JNIEnv,
    _class: jobject,
    surface: jobject,
) -> jlong {
    use ndk_sys::ANativeWindow_fromSurface;
    let window = ANativeWindow_fromSurface(env as *mut _, surface as *mut _);
    window as jlong
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_vello_android_VelloEngine_initLogger(
    _env: *mut JNIEnv,
    _class: jobject,
) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
}
