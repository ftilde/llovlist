use std::fs;
use std::path::PathBuf;

pub fn load(key: &str) -> Option<String> {
    fs::read_to_string(directory().join(key)).ok()
}

pub fn save(key: &str, value: &str) {
    let directory = directory();
    let _ = fs::create_dir_all(&directory);
    let _ = fs::write(directory.join(key), value);
}

pub fn random_u64() -> u64 {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).expect("the OS refused to provide randomness");
    u64::from_le_bytes(bytes)
}

#[cfg(target_os = "android")]
fn directory() -> PathBuf {
    use jni::objects::{JObject, JString};

    // tao hands the activity and JavaVM to ndk_context while setting up the webview.
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.expect("android java vm");
    let mut env = vm.attach_current_thread().expect("attach to the java vm");
    let activity = unsafe { JObject::from_raw(context.context().cast()) };

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

#[cfg(not(target_os = "android"))]
fn directory() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("1kauf")
}
