use crate::config::Config;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jint, jstring};
use jni::{JNIEnv, JavaVM};
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

/// Konteks Android + kelas NodeController untuk memanggil NodeService
/// dari thread core (start/stop Node di proses :node).
struct AndroidContext {
    vm: JavaVM,
    context: GlobalRef,
    node_controller: GlobalRef,
}

static ANDROID_CTX: OnceLock<Mutex<Option<AndroidContext>>> = OnceLock::new();

fn android_ctx_slot() -> &'static Mutex<Option<AndroidContext>> {
    ANDROID_CTX.get_or_init(|| Mutex::new(None))
}

/// Dipanggil Kotlin (CoreBridge.start) dengan applicationContext sebelum
/// nativeStart. Mengembalikan 0 bila sukses.
#[no_mangle]
pub extern "system" fn Java_com_webforge_mobile_CoreBridge_nativeSetContext(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
) -> jint {
    let vm = match env.get_java_vm() {
        Ok(vm) => vm,
        Err(_) => return -1,
    };
    let context_ref = match env.new_global_ref(&context) {
        Ok(r) => r,
        Err(_) => return -1,
    };
    let controller_class = match env.find_class("com/webforge/mobile/NodeController") {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let controller_ref = match env.new_global_ref(&controller_class) {
        Ok(r) => r,
        Err(_) => return -1,
    };
    if let Ok(mut slot) = android_ctx_slot().lock() {
        *slot = Some(AndroidContext {
            vm,
            context: context_ref,
            node_controller: controller_ref,
        });
        0
    } else {
        -1
    }
}

/// Mulai NodeService (:node) lewat NodeController.start. NodeController
/// adalah Kotlin object; metode instance-nya dipanggil lewat field
/// INSTANCE. Mengembalikan true bila Intent berhasil dikirim.
pub fn node_start(args: &[String], cwd: &str, log_path: &str, status_path: &str) -> bool {
    let slot = match android_ctx_slot().lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let ctx = match slot.as_ref() {
        Some(c) => c,
        None => return false,
    };
    let mut env = match ctx.vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return false,
    };
    let class: &JClass = ctx.node_controller.as_obj().into();
    let instance = match env.get_static_field(&class, "INSTANCE", "Lcom/webforge/mobile/NodeController;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    let string_class = match env.find_class("java/lang/String") {
        Ok(c) => c,
        Err(_) => return false,
    };
    let args_array = match env.new_object_array(args.len() as i32, &string_class, JObject::null()) {
        Ok(a) => a,
        Err(_) => return false,
    };
    for (i, arg) in args.iter().enumerate() {
        match env.new_string(arg) {
            Ok(s) => {
                if env.set_object_array_element(&args_array, i as i32, &s).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    let cwd_s = match env.new_string(cwd) { Ok(s) => s, Err(_) => return false };
    let log_s = match env.new_string(log_path) { Ok(s) => s, Err(_) => return false };
    let status_s = match env.new_string(status_path) { Ok(s) => s, Err(_) => return false };
    let result = env.call_method(
        &instance,
        "start",
        "(Landroid/content/Context;[Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Z",
        &[
            JValue::Object(ctx.context.as_obj()),
            JValue::Object(&args_array),
            JValue::Object(&cwd_s),
            JValue::Object(&log_s),
            JValue::Object(&status_s),
        ],
    );
    match result {
        Ok(v) => v.z().unwrap_or(false),
        Err(_) => false,
    }
}

/// Hentikan NodeService lewat NodeController.stop(Context).
pub fn node_stop() -> bool {
    let slot = match android_ctx_slot().lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let ctx = match slot.as_ref() {
        Some(c) => c,
        None => return false,
    };
    let mut env = match ctx.vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return false,
    };
    let class: &JClass = ctx.node_controller.as_obj().into();
    let instance = match env.get_static_field(&class, "INSTANCE", "Lcom/webforge/mobile/NodeController;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    let result = env.call_method(
        &instance,
        "stop",
        "(Landroid/content/Context;)Z",
        &[JValue::Object(ctx.context.as_obj())],
    );
    match result {
        Ok(v) => v.z().unwrap_or(false),
        Err(_) => false,
    }
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
