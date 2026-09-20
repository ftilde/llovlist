//! What the app needs from the host OS beyond the webview.

use std::path::PathBuf;

#[cfg(target_os = "android")]
mod android {
    use std::path::PathBuf;

    use jni::objects::{JObject, JString, JValue};
    use jni::{AttachGuard, JavaVM};

    /// The activity and an attached JNI environment.
    fn activity() -> (JavaVM, JObject<'static>) {
        // tao hands the activity and JavaVM to ndk_context while setting up the webview.
        let context = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.expect("android java vm");
        let activity = unsafe { JObject::from_raw(context.context().cast()) };
        (vm, activity)
    }

    fn attach(vm: &JavaVM) -> AttachGuard<'_> {
        vm.attach_current_thread().expect("attach to the java vm")
    }

    pub fn data_dir() -> PathBuf {
        let (vm, activity) = activity();
        let mut env = attach(&vm);
        let files = env
            .call_method(&activity, "getFilesDir", "()Ljava/io/File;", &[])
            .and_then(|value| value.l())
            .expect("Context.getFilesDir");
        let path = env
            .call_method(&files, "getAbsolutePath", "()Ljava/lang/String;", &[])
            .and_then(|value| value.l())
            .expect("File.getAbsolutePath");
        let path: String = env
            .get_string(&JString::from(path))
            .expect("path is a string")
            .into();
        PathBuf::from(path)
    }

    pub fn show_when_locked(on: bool) {
        let (vm, activity) = activity();
        let mut env = attach(&vm);
        // An exception here (wrong thread, old API) is a missing convenience, not a crash.
        if env
            .call_method(
                &activity,
                "setShowWhenLocked",
                "(Z)V",
                &[JValue::Bool(on.into())],
            )
            .is_err()
        {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            eprintln!("could not set showWhenLocked");
        }
    }
}

/// Where this app keeps its files.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "android")]
    return android::data_dir();
    #[cfg(not(target_os = "android"))]
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("llovlist")
}

/// Keeps the app visible over the lock screen, where the OS supports it.
pub fn show_when_locked(on: bool) {
    #[cfg(target_os = "android")]
    android::show_when_locked(on);
    #[cfg(not(target_os = "android"))]
    let _ = on;
}
