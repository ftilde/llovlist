mod drag;
mod notice;
mod platform;
mod state;
mod storage;
mod sync;
mod views;

use dioxus::prelude::*;
use llovlist_core::{ListMut, TrimStr};

use crate::notice::{Notice, Notices};
use crate::state::{
    Scope, Settings, State, create_list, edit, fill_from, remove_list, restore_deleted,
};
use crate::sync::start_sync;
use crate::views::lists::ListsView;
use crate::views::settings::SettingsView;
use crate::views::shop::ShopView;

const CSS: &str = include_str!("../assets/main.css");

// dioxus re-exports the same webview crate under two names, one per feature.
#[cfg(all(feature = "desktop", not(feature = "mobile")))]
use dioxus::desktop as webview;
#[cfg(feature = "mobile")]
use dioxus::mobile as webview;

fn main() {
    #[cfg(feature = "desktop")]
    {
        // GTK ignores xdg-decoration, so a tiling WM cannot remove the title bar itself.
        let window = webview::WindowBuilder::new()
            .with_decorations(false)
            .with_title("llovlist");
        dioxus::LaunchBuilder::desktop()
            .with_cfg(webview_config().with_window(window))
            .launch(App);
    }
    #[cfg(all(feature = "mobile", not(feature = "desktop")))]
    dioxus::LaunchBuilder::mobile()
        .with_cfg(webview_config())
        .launch(App);
    #[cfg(not(any(feature = "desktop", feature = "mobile")))]
    dioxus::launch(App);
}

#[cfg(any(feature = "desktop", feature = "mobile"))]
fn webview_config() -> webview::Config {
    // Release builds otherwise preventDefault every contextmenu event, taking the
    // long-press paste menu with it.
    webview::Config::new().with_disable_context_menu(false)
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Lists,
    Templates,
    Settings,
    Shopping,
}

impl View {
    fn scope(self) -> Scope {
        match self {
            Self::Templates => Scope::Templates,
            _ => Scope::Lists,
        }
    }
}

fn enter_shop(mut settings: Signal<Settings>, mut view: Signal<View>, list: TrimStr) {
    settings.write().shopping = Some(list);
    settings.read().save();
    view.set(View::Shopping);
    platform::show_when_locked(true);
}

fn leave_shop(mut settings: Signal<Settings>, mut view: Signal<View>) {
    settings.write().shopping = None;
    settings.read().save();
    view.set(View::Lists);
    platform::show_when_locked(false);
}

#[component]
fn App() -> Element {
    let settings = use_context_provider(|| Signal::new(Settings::load()));
    let state = use_context_provider(|| Signal::new(State::load(settings.read().device)));
    let notice = use_context_provider(|| Signal::new(Notice::default()));
    let mut view = use_signal(|| View::Lists);
    let mut menu_open = use_signal(|| false);
    let mut adding = use_signal(|| false);
    let mut filling = use_signal(|| false);
    let mut new_name = use_signal(String::new);
    let current_list = use_signal(|| {
        let remembered = settings.read().shopping.clone();
        remembered.or_else(|| state.read().names(Scope::Lists).first().cloned())
    });
    let current_template = use_signal(|| None::<TrimStr>);
    // A relaunch mid-shop goes straight back in, still over the lock screen.
    use_hook(|| {
        // Bound first: the read guard of an `if let` scrutinee would outlive the write below.
        let remembered = settings.read().shopping.clone();
        if let Some(list) = remembered
            && state.read().data.lists.has(&list)
        {
            enter_shop(settings, view, list);
        }
    });

    let scope = view().scope();
    let current = match scope {
        Scope::Lists => current_list,
        Scope::Templates => current_template,
    };
    let names = state.read().names(scope);
    let restorable = scope.of(&state.read().data).restorable().cloned();
    let templates = state.read().names(Scope::Templates);
    let noun = match scope {
        Scope::Lists => "list",
        Scope::Templates => "template",
    };

    // Falling back to the first name keeps a list selected when the current one is
    // deleted here or disappears in a merge.
    let selected = current()
        .filter(|list| names.contains(list))
        .or_else(|| names.first().cloned());
    let can_undo = selected
        .as_ref()
        .is_some_and(|list| scope.of(&state.read().data).can_undo(list));
    let can_redo = selected
        .as_ref()
        .is_some_and(|list| scope.of(&state.read().data).can_redo(list));

    let for_selected = |f: fn(&mut ListMut)| {
        let selected = selected.clone();
        move |_| {
            if let Some(list) = selected.clone() {
                edit(state, scope, list, f);
            }
        }
    };

    let mut add = move || {
        if let Ok(name) = TrimStr::new(&new_name()) {
            create_list(state, scope, name.clone());
            let mut current = current;
            current.set(Some(name));
        }
        new_name.set(String::new());
        adding.set(false);
    };

    rsx! {
        document::Style { {CSS} }
        div { class: if scope == Scope::Templates { "app templates" } else { "app" },
            header { class: "topbar",
                if view() == View::Settings {
                    button { class: "icon", title: "back", onclick: move |_| view.set(View::Lists), "←" }
                    h1 { "settings" }
                } else if view() == View::Shopping {
                    button { class: "icon", title: "back", onclick: move |_| leave_shop(settings, view), "←" }
                    h1 { if let Some(list) = selected.clone() { "{list}" } }
                    button { class: "icon", title: "undo", disabled: !can_undo, onclick: for_selected(|list| list.undo()), "↶" }
                    button { class: "icon", title: "redo", disabled: !can_redo, onclick: for_selected(|list| list.redo()), "↷" }
                    button { class: "icon", title: "sync", onclick: move |_| start_sync(state, settings, notice), "⟳" }
                } else {
                    if view() == View::Templates {
                        button { class: "icon", title: "back", onclick: move |_| view.set(View::Lists), "←" }
                        h1 { "templates" }
                    } else {
                        h1 { "llovlist" }
                    }
                    button { class: "icon", title: "undo", disabled: !can_undo, onclick: for_selected(|list| list.undo()), "↶" }
                    button { class: "icon", title: "redo", disabled: !can_redo, onclick: for_selected(|list| list.redo()), "↷" }
                    button { class: "icon", title: "sync", onclick: move |_| start_sync(state, settings, notice), "⟳" }
                    if view() == View::Lists {
                        button {
                            class: "icon",
                            title: "go shopping",
                            disabled: selected.is_none(),
                            onclick: {
                                let selected = selected.clone();
                                move |_| {
                                    if let Some(list) = selected.clone() {
                                        enter_shop(settings, view, list);
                                    }
                                }
                            },
                            "🛒"
                        }
                    }
                    button {
                        class: if menu_open() { "icon active" } else { "icon" },
                        title: "menu",
                        onclick: move |_| menu_open.toggle(),
                        "☰"
                    }
                }
            }

            if menu_open() {
                div { class: "scrim", onclick: move |_| menu_open.set(false) }
                nav { class: "menu",
                    button { onclick: move |_| { menu_open.set(false); adding.set(true); }, "add {noun}" }
                    if let Some(list) = selected.clone() {
                        button {
                            onclick: move |_| { menu_open.set(false); remove_list(state, scope, &list); },
                            "delete {noun} “{list}”"
                        }
                    }
                    if let Some(list) = restorable {
                        button {
                            onclick: move |_| { menu_open.set(false); restore_deleted(state, scope); },
                            "restore {noun} “{list}”"
                        }
                    }
                    if view() == View::Lists {
                        button {
                            disabled: selected.is_none(),
                            onclick: move |_| { menu_open.set(false); filling.set(true); },
                            "fill from template…"
                        }
                        button { onclick: move |_| { menu_open.set(false); view.set(View::Templates); }, "templates" }
                    }
                    button { onclick: move |_| { menu_open.set(false); view.set(View::Settings); }, "settings" }
                }
            }

            match (view(), selected.clone()) {
                (View::Settings, _) => rsx! { SettingsView {} },
                (View::Shopping, Some(list)) => rsx! { ShopView { list } },
                (_, selected) => rsx! { ListsView { scope, selected, current } },
            }

            Notices {}
        }

        if adding() {
            div { class: "overlay", onclick: move |_| adding.set(false),
                div { class: "card", onclick: move |event| event.stop_propagation(),
                    h2 { "new {noun}" }
                    input {
                        placeholder: "name",
                        autofocus: true,
                        value: "{new_name}",
                        // Without this Android offers Next, which moves focus to the next
                        // field instead of sending Enter.
                        enterkeyhint: "done",
                        oninput: move |event| new_name.set(event.value()),
                        onkeydown: move |event: KeyboardEvent| match event.key() {
                            Key::Enter => add(),
                            Key::Escape => adding.set(false),
                            _ => {}
                        },
                    }
                    div { class: "actions",
                        button { onclick: move |_| adding.set(false), "cancel" }
                        button { class: "primary", onclick: move |_| add(), "add" }
                    }
                }
            }
        }

        if let (true, Some(list)) = (filling(), selected.clone()) {
            div { class: "overlay", onclick: move |_| filling.set(false),
                div { class: "card", onclick: move |event| event.stop_propagation(),
                    h2 { "fill “{list}” from" }
                    if templates.is_empty() {
                        p { class: "empty", "no templates yet" }
                    }
                    for template in templates.clone() {
                        button {
                            key: "{template}",
                            onclick: {
                                let list = list.clone();
                                move |_| {
                                    fill_from(state, list.clone(), &template);
                                    filling.set(false);
                                }
                            },
                            "{template}"
                        }
                    }
                    div { class: "actions",
                        button { onclick: move |_| filling.set(false), "cancel" }
                    }
                }
            }
        }
    }
}
