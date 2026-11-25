use crate::config::Config;
use jni::objects::{JClass, JString};
use jni::sys::{jint, jstring};
use jni::JNIEnv;
use std::sync::{Mutex, OnceLock};

struct RunningCore {
    token: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

static CORE: OnceLock<Mutex<Option<RunningCore>>> = OnceLock::new();

fn core_slot() -> &'static Mutex<Option<RunningCore>> {
    CORE.get_or_init(|| Mutex::new(None))
}

/// Simpan nativeLibraryDir Android ke state core agar deteksi PHP
/// menemukan libphp.so. Dipanggil Kotlin sebelum nativeStart.
/// Mengembalikan 0 bila sukses, -1 bila string tidak terbaca.
#[no_mangle]
pub extern "system" fn Java_com_webforge_mobile_CoreBridge_nativeSetNativeLibDir(
    mut env: JNIEnv,
    _class: JClass,
    dir: JString,
) -> jint {
    let dir: String = match env.get_string(&dir) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    crate::state::set_native_lib_dir(std::path::PathBuf::from(dir));
    0
}

#[no_mangle]
pub extern "system" fn Java_com_webforge_mobile_CoreBridge_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
    workspace_dir: JString,
    port: jint,
) -> jint {
    let data_dir: String = match env.get_string(&data_dir) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    let workspace_dir: String = match env.get_string(&workspace_dir) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    let mut slot = match core_slot().lock() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    if slot.is_some() {
        return 0; // already running
    }
    let config = match Config::new(
        std::path::PathBuf::from(data_dir),
        std::path::PathBuf::from(workspace_dir),
        port as u16,
    ) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let token = config.token.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let thread = std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let _ = crate::run_with_shutdown(config, rx).await;
        });
    });
    *slot = Some(RunningCore { token, shutdown: Some(tx), thread: Some(thread) });
    0
}

#[no_mangle]
pub extern "system" fn Java_com_webforge_mobile_CoreBridge_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    let mut slot = match core_slot().lock() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    if let Some(mut core) = slot.take() {
        if let Some(tx) = core.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = core.thread.take() {
            let _ = t.join();
        }
    }
    0
}

#[no_mangle]
pub extern "system" fn Java_com_webforge_mobile_CoreBridge_nativeGetToken(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    let token = core_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|c| c.token.clone()))
        .unwrap_or_default();
    match env.new_string(&token) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
