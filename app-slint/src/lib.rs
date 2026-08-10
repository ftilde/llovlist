mod storage;
mod sync;

use std::sync::{Arc, Mutex};

use kauf_core::{DeviceId, Edit, ItemName, Session, ShoppingList, Tag};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

const LIST_KEY: &str = "list.json";
const SETTINGS_KEY: &str = "settings.json";

#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    server: String,
    list: String,
    token: String,
    device: DeviceId,
}

impl Settings {
    fn load() -> Self {
        storage::load(SETTINGS_KEY).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or(
            Self {
                server: String::new(),
                list: "home".to_owned(),
                token: String::new(),
                device: DeviceId::new(storage::random_u64()),
            },
        )
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            storage::save(SETTINGS_KEY, &json);
        }
    }
}

fn load_list() -> ShoppingList {
    storage::load(LIST_KEY).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

fn store(session: &Session) {
    if let Ok(json) = serde_json::to_string(session.list()) {
        storage::save(LIST_KEY, &json);
    }
}

fn init_logging() {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("kauf"),
    );
    #[cfg(not(target_os = "android"))]
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
}

fn refresh(window: &MainWindow, session: &Session) {
    let items: Vec<ItemData> = session
        .list()
        .items()
        .into_iter()
        .map(|(name, tags)| {
            let tags: Vec<SharedString> = tags.iter().map(|tag| tag.to_string().into()).collect();
            ItemData { name: name.to_string().into(), tags: ModelRc::new(VecModel::from(tags)) }
        })
        .collect();
    window.set_items(ModelRc::new(VecModel::from(items)));
    window.set_can_undo(session.can_undo());
}

pub fn run() -> Result<(), slint::PlatformError> {
    init_logging();
    let settings = Arc::new(Mutex::new(Settings::load()));
    let session = {
        let device = settings.lock().expect("settings lock").device;
        Arc::new(Mutex::new(Session::new(device, load_list())))
    };

    let window = MainWindow::new()?;
    {
        let settings = settings.lock().expect("settings lock");
        window.set_server(settings.server.clone().into());
        window.set_list_name(settings.list.clone().into());
        window.set_token(settings.token.clone().into());
        window.set_device(format!("{:?}", settings.device).into());
    }
    refresh(&window, &session.lock().expect("session lock"));

    let edit = {
        let session = session.clone();
        let weak = window.as_weak();
        move |edit: Edit| {
            let mut current = session.lock().expect("session lock");
            current.edit(edit);
            store(&current);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &current);
            }
        }
    };

    window.on_add_item({
        let edit = edit.clone();
        move |text| {
            if let Ok(name) = ItemName::new(&text) {
                edit(Edit::add_item(name));
            }
        }
    });

    window.on_remove_item({
        let edit = edit.clone();
        move |name| {
            if let Ok(name) = ItemName::new(&name) {
                edit(Edit::RemoveItem { name });
            }
        }
    });

    window.on_add_tag({
        let edit = edit.clone();
        move |name, tag| {
            if let (Ok(name), Ok(tag)) = (ItemName::new(&name), Tag::new(&tag)) {
                edit(Edit::AddTag { name, tag });
            }
        }
    });

    window.on_remove_tag({
        let edit = edit.clone();
        move |name, tag| {
            if let (Ok(name), Ok(tag)) = (ItemName::new(&name), Tag::new(&tag)) {
                edit(Edit::RemoveTag { name, tag });
            }
        }
    });

    window.on_undo({
        let session = session.clone();
        let weak = window.as_weak();
        move || {
            let mut current = session.lock().expect("session lock");
            current.undo();
            store(&current);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &current);
            }
        }
    });

    window.on_save_settings({
        let settings = settings.clone();
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let mut settings = settings.lock().expect("settings lock");
            settings.server = window.get_server().into();
            settings.list = window.get_list_name().into();
            settings.token = window.get_token().into();
            settings.save();
        }
    });

    window.on_sync({
        let session = session.clone();
        let settings = settings.clone();
        let weak = window.as_weak();
        move || {
            let config = settings.lock().expect("settings lock").clone();
            let local = session.lock().expect("session lock").list().clone();
            if let Some(window) = weak.upgrade() {
                window.set_status("syncing…".into());
                window.set_error("".into());
            }

            let session = session.clone();
            let weak = weak.clone();
            std::thread::spawn(move || {
                let merged = sync::exchange(&config.server, &config.list, &config.token, local);
                let _ = weak.upgrade_in_event_loop(move |window| {
                    let mut current = session.lock().expect("session lock");
                    match merged {
                        Ok(merged) => {
                            current.merge(merged);
                            store(&current);
                            let count = current.list().items().len();
                            let plural = if count == 1 { "" } else { "s" };
                            log::info!("sync complete, {count} items");
                            window.set_status(format!("synced, {count} item{plural}").into());
                        }
                        Err(error) => {
                            log::error!("sync failed: {error}");
                            window.set_status("sync failed".into());
                            window.set_error(error.into());
                        }
                    }
                    refresh(&window, &current);
                });
            });
        }
    });

    window.run()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    if let Some(path) = app.internal_data_path() {
        storage::remember(path);
    }
    slint::android::init(app).expect("start the android backend");
    run().expect("run the app");
}
