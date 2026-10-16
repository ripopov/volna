//! GPUI frontend for viewing VTR recordings.

pub mod app;
pub mod assets;
mod canvas;
#[cfg(not(target_family = "wasm"))]
pub mod desktop;
mod dock;
mod frame_stats;
mod islands;
#[cfg(not(target_family = "wasm"))]
mod native_import;
#[cfg(not(target_family = "wasm"))]
mod native_import_view;
#[cfg(not(target_family = "wasm"))]
pub mod native_workspace;
mod palette;
mod settings_json;
mod settings_panel;
mod sidebar;
mod start_panel;
#[cfg(target_arch = "wasm32")]
mod table_clipboard;
mod table_columns;
mod table_filters;
mod table_panel;
pub mod theme;
mod traces;
mod transaction_panel;
mod ui;
#[cfg(target_family = "wasm")]
mod web_remote;

use gpui_kit::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, point, px, size,
};

pub use app::Workspace;

pub fn init_app(cx: &mut App) {
    gpui_kit::init(cx);
    // Test applications have short-lived executors; core tests use their own worker threads.
    #[cfg(not(test))]
    {
        let executor = cx.background_executor().clone();
        volna_core::table::query::set_executor(move |work| {
            executor.spawn(async move { work() }).detach();
        });
    }
    table_panel::init(cx);
    table_filters::init(cx);
    table_columns::init(cx);
    theme::set(theme::CoreTheme::islands_dark(), cx);
    if let Err(e) = assets::load_fonts(cx) {
        log::warn!("failed to load bundled fonts: {e:#}");
    }
    app::init(cx);
}

const WINDOW_DECORATIONS_ENV: &str = "VOLNA_WINDOW_DECORATIONS";

fn window_decorations(env: Option<&str>) -> WindowDecorations {
    match env.map(str::trim) {
        Some("server") => WindowDecorations::Server,
        None | Some("" | "client") => WindowDecorations::Client,
        Some(other) => {
            log::warn!("{WINDOW_DECORATIONS_ENV}={other:?}: expected `client` or `server`");
            WindowDecorations::Client
        }
    }
}

fn window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
    let decorations = std::env::var(WINDOW_DECORATIONS_ENV).ok();
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Volna".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(10.0), px(10.0))),
        }),
        app_owns_titlebar_drag: true,
        window_decorations: Some(window_decorations(decorations.as_deref())),
        window_min_size: Some(size(px(800.0), px(500.0))),
        #[cfg(not(target_family = "wasm"))]
        app_id: Some(desktop::APP_ID.into()),
        #[cfg(target_os = "linux")]
        icon: Some(desktop::window_icon()),
        ..Default::default()
    }
}

#[must_use]
pub fn application() -> Application {
    gpui_kit::application().with_assets(assets::Assets)
}

pub fn open_main_window(
    cx: &mut App,
    embedded: bool,
) -> anyhow::Result<gpui_kit::Entity<Workspace>> {
    let options = if embedded {
        WindowOptions::default()
    } else {
        window_options(cx)
    };
    let (_, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
        let ws = cx.new(|cx| {
            let mut w = Workspace::new(window, cx);
            w.embedded = embedded;
            #[cfg(target_family = "wasm")]
            {
                use volna_core::settings::Host;
                w.app
                    .configure_settings(if embedded { Host::Vscode } else { Host::Web }, None);
                if !embedded && let Some(text) = web::read_settings() {
                    w.app.settings_loaded(&text);
                }
            }
            w
        });
        #[cfg(not(target_family = "wasm"))]
        {
            let weak = ws.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                weak.update(cx, |ws, cx| {
                    ws.app.persist_workspace(volna_core::Instant::now(), true);
                    ws.after(Some(window), cx);
                    let scheduler = &ws.app.workspace.scheduler;
                    !scheduler.enabled()
                        || scheduler.suspended()
                        || scheduler.target().is_none()
                        || (!scheduler.dirty() && scheduler.outstanding().is_none())
                })
                .unwrap_or(true)
            });
        }
        ws
    })?;
    Ok(workspace)
}

#[cfg(target_family = "wasm")]
pub mod web {

    use std::cell::RefCell;

    use futures::StreamExt;
    use futures::channel::mpsc;
    use wasm_bindgen::prelude::*;

    enum HostEvent {
        Open(String, Vec<u8>),
        Resource(volna_trace::session::OpenSpec, Box<OpenMetadata>),
        RemoteFrame(String, Vec<u8>),
        RemoteError(String, String),
        RemoteContinue(String),
        Workspace(WorkspaceMessage),
        Theme(Box<crate::theme::CoreTheme>),
        Settings(std::collections::BTreeMap<String, volna_core::settings::Value>),
        Command(volna_core::app::Command),
        DebugState,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OpenMetadata {
        trace_uri: String,
        candidates: Option<Candidates>,
        #[serde(default)]
        settings: serde_json::Map<String, serde_json::Value>,
    }
    #[derive(serde::Deserialize)]
    struct Candidates {
        sidecar: volna_core::workspace::persistence::Candidate,
        fallback: volna_core::workspace::persistence::Candidate,
    }

    fn host_settings(
        map: &serde_json::Map<String, serde_json::Value>,
    ) -> std::collections::BTreeMap<String, volna_core::settings::Value> {
        use volna_core::settings::Value;
        map.iter()
            .filter_map(|(key, value)| {
                let value = match value {
                    serde_json::Value::Bool(b) => Value::Bool(*b),
                    serde_json::Value::Number(n) => n
                        .as_i64()
                        .map(Value::Integer)
                        .or_else(|| n.as_f64().map(Value::Number))?,
                    serde_json::Value::String(s) => Value::Text(s.clone()),
                    _ => return None,
                };
                Some((key.clone(), value))
            })
            .collect()
    }
    #[derive(serde::Deserialize)]
    #[serde(tag = "type", rename_all = "camelCase")]
    enum WorkspaceMessage {
        Saved {
            ticket: volna_core::workspace::persistence::SaveTicket,
            error: Option<String>,
        },
        Workspace {
            candidate: volna_core::workspace::persistence::Candidate,
        },
        WorkspaceDestination {
            target: volna_core::workspace::persistence::Target,
        },
        RequestWorkspace,
        Candidates {
            trace_uri: String,
            candidates: Candidates,
        },
    }

    thread_local! {
        static CANDIDATES: RefCell<Option<(String, Candidates)>> = const { RefCell::new(None) };
        static PENDING_THEME: RefCell<Option<crate::theme::CoreTheme>> = const { RefCell::new(None) };
        static HOST_TX: RefCell<Option<mpsc::UnboundedSender<HostEvent>>> = const { RefCell::new(None) };
    }

    #[wasm_bindgen]
    pub fn open_trace(name: String, bytes: Vec<u8>) {
        enqueue(HostEvent::Open(name, bytes)).ok();
    }

    fn enqueue(event: HostEvent) -> Result<(), JsValue> {
        HOST_TX.with(|tx| {
            tx.borrow()
                .as_ref()
                .ok_or_else(|| JsValue::from_str("viewer is not ready"))?
                .unbounded_send(event)
                .map_err(|_| JsValue::from_str("viewer is closed"))
        })
    }

    #[wasm_bindgen]
    pub fn open_resource(name: String, bytes: Vec<u8>, metadata: &str) -> Result<(), JsValue> {
        let metadata = serde_json::from_str(metadata)
            .map_err(|e| JsValue::from_str(&format!("invalid open metadata: {e}")))?;
        enqueue(HostEvent::Resource(
            volna_trace::session::OpenSpec::Bytes { name, bytes },
            Box::new(metadata),
        ))
    }

    #[wasm_bindgen]
    pub fn open_remote(name: String, metadata: &str) -> Result<(), JsValue> {
        let metadata: OpenMetadata = serde_json::from_str(metadata)
            .map_err(|e| JsValue::from_str(&format!("invalid open metadata: {e}")))?;
        enqueue(HostEvent::Resource(
            volna_trace::session::OpenSpec::Remote {
                name,
                limits: volna_trace::remote::limits::Limits::default(),
            },
            Box::new(metadata),
        ))
    }

    #[wasm_bindgen]
    pub fn set_settings(json: &str) -> Result<(), JsValue> {
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json)
            .map_err(|e| JsValue::from_str(&format!("invalid settings: {e}")))?;
        enqueue(HostEvent::Settings(host_settings(&map)))
    }

    fn local_storage() -> Option<js_sys::Object> {
        let global = js_sys::global();
        js_sys::Reflect::get(&global, &JsValue::from_str("localStorage"))
            .ok()?
            .dyn_into::<js_sys::Object>()
            .ok()
    }

    const STORAGE_KEY: &str = "volna.settings";

    pub fn read_settings() -> Option<String> {
        let storage = local_storage()?;
        let get = js_sys::Reflect::get(&storage, &JsValue::from_str("getItem")).ok()?;
        get.dyn_ref::<js_sys::Function>()?
            .call1(&storage, &JsValue::from_str(STORAGE_KEY))
            .ok()?
            .as_string()
    }

    pub fn write_settings(bytes: &[u8]) -> Result<(), String> {
        let storage = local_storage().ok_or("localStorage is unavailable")?;
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let set = js_sys::Reflect::get(&storage, &JsValue::from_str("setItem"))
            .map_err(|_| "localStorage.setItem is unavailable".to_owned())?;
        set.dyn_ref::<js_sys::Function>()
            .ok_or("localStorage.setItem is not a function")?
            .call2(
                &storage,
                &JsValue::from_str(STORAGE_KEY),
                &JsValue::from_str(text),
            )
            .map(|_| ())
            .map_err(|_| "localStorage write failed".to_owned())
    }

    pub fn open_host_settings() {
        post(serde_json::json!({"type":"openSettings"}));
    }
    #[wasm_bindgen]
    pub fn trace_frame(connection: String, bytes: Vec<u8>) -> Result<(), JsValue> {
        enqueue(HostEvent::RemoteFrame(connection, bytes))
    }
    #[wasm_bindgen]
    pub fn trace_error(connection: String, message: String) -> Result<(), JsValue> {
        enqueue(HostEvent::RemoteError(connection, message))
    }
    #[wasm_bindgen]
    pub fn trace_continue(connection: String) -> Result<(), JsValue> {
        enqueue(HostEvent::RemoteContinue(connection))
    }

    #[wasm_bindgen]
    pub fn workspace_message(json: &str) -> Result<(), JsValue> {
        let message = serde_json::from_str(json)
            .map_err(|e| JsValue::from_str(&format!("invalid workspace message: {e}")))?;
        enqueue(HostEvent::Workspace(message))
    }

    pub fn request_workspace_candidates(trace_uri: &str) {
        CANDIDATES.with(|slot| {
            if slot
                .borrow()
                .as_ref()
                .is_some_and(|(uri, _)| uri == trace_uri)
                && let Some((trace_uri, candidates)) = slot.borrow_mut().take()
            {
                enqueue(HostEvent::Workspace(WorkspaceMessage::Candidates {
                    trace_uri,
                    candidates,
                }))
                .ok();
            }
        });
    }

    fn post(message: serde_json::Value) {
        let global = js_sys::global();
        if let Ok(callback) = js_sys::Reflect::get(&global, &JsValue::from_str("volnaWorkspace"))
            && let Some(callback) = callback.dyn_ref::<js_sys::Function>()
        {
            callback
                .call1(&global, &JsValue::from_str(&message.to_string()))
                .ok();
        }
    }

    pub fn write_workspace(ticket: volna_core::workspace::persistence::SaveTicket, bytes: Vec<u8>) {
        post(
            serde_json::json!({"type":"workspace", "ticket":ticket, "json":String::from_utf8(bytes).expect("core emits UTF-8 JSON")}),
        );
    }
    pub fn workspace_dialog(save: bool) {
        post(serde_json::json!({"type": if save { "saveWorkspaceAs" } else { "openWorkspace" }}));
    }
    pub fn trace_closed(trace_uri: &str) {
        post(serde_json::json!({"type":"closeTrace", "traceUri":trace_uri}));
    }
    pub fn notice(text: &str) {
        post(serde_json::json!({"type":"notice", "text":text}));
    }

    pub fn viewer_state(state: &str) {
        let global = js_sys::global();
        if let Ok(callback) = js_sys::Reflect::get(&global, &JsValue::from_str("volnaViewerState"))
            && let Some(callback) = callback.dyn_ref::<js_sys::Function>()
        {
            callback.call1(&global, &JsValue::from_str(state)).ok();
        }
    }

    pub fn announce(text: &str) {
        use js_sys::{Array, Function, Reflect};
        let call = |target: &JsValue, name: &str, args: &Array| {
            let f = Reflect::get(target, &JsValue::from_str(name)).ok()?;
            f.dyn_ref::<Function>()?.apply(target, args).ok()
        };
        let Ok(document) = Reflect::get(&js_sys::global(), &JsValue::from_str("document")) else {
            return;
        };
        let text = JsValue::from_str(text);
        if call(&document, "ariaNotify", &Array::of1(&text)).is_some() {
            return;
        }
        let id = JsValue::from_str("volna-announce");
        let region = match call(&document, "getElementById", &Array::of1(&id)) {
            Some(region) if !region.is_null() => region,
            _ => {
                let Some(region) = call(
                    &document,
                    "createElement",
                    &Array::of1(&JsValue::from_str("div")),
                ) else {
                    return;
                };
                for (name, value) in [
                    ("id", "volna-announce"),
                    ("role", "status"),
                    ("aria-live", "polite"),
                    (
                        "style",
                        "position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0)",
                    ),
                ] {
                    call(
                        &region,
                        "setAttribute",
                        &Array::of2(&JsValue::from_str(name), &JsValue::from_str(value)),
                    );
                }
                let Ok(body) = Reflect::get(&document, &JsValue::from_str("body")) else {
                    return;
                };
                call(&body, "appendChild", &Array::of1(&region));
                region
            }
        };
        Reflect::set(&region, &JsValue::from_str("textContent"), &text).ok();
    }

    #[wasm_bindgen]
    pub fn dispatch_command(name: &str) -> Result<(), JsValue> {
        let command = volna_core::app::Command::named(name)
            .ok_or_else(|| JsValue::from_str("unknown Volna command"))?;
        enqueue(HostEvent::Command(command))
    }

    #[wasm_bindgen]
    pub fn debug_state() {
        enqueue(HostEvent::DebugState).ok();
    }

    #[wasm_bindgen]
    pub fn set_theme(json: &str) -> Result<(), JsValue> {
        let palette = crate::theme::HostPalette::from_json(json)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        install_theme(crate::theme::CoreTheme::from_host(&palette));
        Ok(())
    }

    #[wasm_bindgen]
    pub fn set_vscode_theme(snapshot: &str) {
        install_theme(crate::theme::CoreTheme::from_host(
            &crate::theme::vscode::host_palette(snapshot),
        ));
    }

    fn install_theme(theme: crate::theme::CoreTheme) {
        HOST_TX.with(|tx| {
            if let Some(tx) = tx.borrow().as_ref() {
                tx.unbounded_send(HostEvent::Theme(Box::new(theme))).ok();
            } else {
                PENDING_THEME.with(|slot| *slot.borrow_mut() = Some(theme));
            }
        });
    }

    pub fn request_open_dialog() {
        let global = js_sys::global();
        if let Ok(f) = js_sys::Reflect::get(&global, &JsValue::from_str("volnaOpen"))
            && let Some(f) = f.dyn_ref::<js_sys::Function>()
        {
            f.call0(&global).ok();
            return;
        }
        log::warn!("volnaOpen is not defined by the host page");
    }

    fn host_snapshot(name: &str) -> Option<String> {
        let global = js_sys::global();
        js_sys::Reflect::get(&global, &name.into())
            .ok()?
            .dyn_ref::<js_sys::Function>()?
            .call0(&global)
            .ok()?
            .as_string()
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        // Worker instances initialize the module, but must not create another application.
        if js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("document"))?.is_undefined() {
            return Ok(());
        }
        let memory = js_sys::WebAssembly::Memory::from(wasm_bindgen::memory());
        let atomics = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Atomics"))?;
        // VS Code hides the global SharedArrayBuffer constructor but exposes shared Wasm memory.
        let buffer = js_sys::Object::from(memory.buffer());
        if buffer.to_string() != "[object SharedArrayBuffer]"
            || !js_sys::Reflect::get(&atomics, &JsValue::from_str("waitAsync"))?.is_function()
        {
            return Err(JsValue::from_str(
                "Volna requires shared WebAssembly memory and Atomics.waitAsync. Serve it over HTTPS with Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp.",
            ));
        }
        console_error_panic_hook::set_once();
        gpui_kit::web::init_logging();
        if let Some(json) = host_snapshot("volnaHostTheme") {
            if let Err(error) = set_theme(&json) {
                log::warn!("invalid initial host theme: {error:?}");
            }
        } else if let Some(snapshot) = host_snapshot("volnaVscodeTheme") {
            set_vscode_theme(&snapshot);
        }
        let (tx, mut rx) = mpsc::unbounded::<HostEvent>();
        let embedded = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("volnaEmbedded"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let handle = super::application().run_embedded(move |cx| {
            super::init_app(cx);
            PENDING_THEME.with(|slot| {
                if let Some(theme) = slot.borrow_mut().take() {
                    crate::theme::install(theme, cx);
                }
            });
            HOST_TX.with(|slot| *slot.borrow_mut() = Some(tx));
            match super::open_main_window(cx, embedded) {
                Ok(workspace) => {
                    let window = cx.windows().first().copied();
                    cx.spawn(async move |cx| {
                        while let Some(event) = rx.next().await {
                            if let HostEvent::Command(
                                command @ (volna_core::app::Command::Undo
                                | volna_core::app::Command::Redo),
                            ) = &event
                                && let Some(window) = window
                            {
                                let keys = if *command == volna_core::app::Command::Undo {
                                    "ctrl-z"
                                } else {
                                    "ctrl-shift-z"
                                };
                                let keystroke =
                                    gpui_kit::Keystroke::parse(keys).expect("a valid keystroke");
                                if gpui_kit::AppContext::update_window(
                                    cx,
                                    window,
                                    |_, window, cx| window.dispatch_keystroke(keystroke, cx),
                                )
                                .unwrap_or(false)
                                {
                                    continue;
                                }
                            }
                            workspace.update(cx, |ws, cx| match event {
                                HostEvent::Open(name, bytes) => ws.open_bytes(name, bytes, cx),
                                HostEvent::RemoteFrame(connection, bytes) => {
                                    ws.remote_frame(&connection, bytes, cx)
                                }
                                HostEvent::RemoteError(connection, message) => {
                                    ws.remote_error(&connection, &message, cx)
                                }
                                HostEvent::RemoteContinue(connection) => {
                                    ws.remote_continue(&connection, cx)
                                }
                                HostEvent::Resource(spec, metadata) => {
                                    use volna_core::settings::Autosave;
                                    use volna_core::workspace::persistence::Persistence;
                                    if ws.embedded {
                                        ws.app.set_host_settings(host_settings(&metadata.settings));
                                    }
                                    let resolved = ws.app.settings.resolved().clone();
                                    let policy = match resolved.workspace.autosave {
                                        Autosave::Sidecar => Persistence::Auto,
                                        Autosave::Vscode => Persistence::Storage,
                                        Autosave::Off => Persistence::Disabled,
                                    };
                                    ws.app.configure_persistence(policy);
                                    let spec = match spec {
                                        volna_trace::session::OpenSpec::Remote { name, .. } => {
                                            volna_trace::session::OpenSpec::Remote {
                                                name,
                                                limits: resolved.limits(),
                                            }
                                        }
                                        spec => spec,
                                    };
                                    let OpenMetadata {
                                        trace_uri,
                                        candidates,
                                        ..
                                    } = *metadata;
                                    CANDIDATES.with(|slot| {
                                        *slot.borrow_mut() =
                                            candidates.map(|c| (trace_uri.clone(), c))
                                    });
                                    ws.app.open_resource(spec, trace_uri);
                                    ws.after(None, cx);
                                }
                                HostEvent::Workspace(message) => {
                                    use volna_core::workspace::persistence::Content;
                                    match message {
                                        WorkspaceMessage::Saved { ticket, error } => {
                                            ws.app.workspace_saved(
                                                ticket,
                                                error,
                                                volna_core::Instant::now(),
                                            )
                                        }
                                        WorkspaceMessage::Candidates {
                                            trace_uri,
                                            candidates,
                                        } => ws.app.restore_candidates(
                                            &trace_uri,
                                            candidates.sidecar,
                                            candidates.fallback,
                                        ),
                                        WorkspaceMessage::Workspace { candidate } => {
                                            let result = match candidate.content {
                                                Content::Bytes(bytes) => {
                                                    ws.app.open_workspace(candidate.target, &bytes)
                                                }
                                                Content::Missing => Err(anyhow::anyhow!(
                                                    "workspace file does not exist"
                                                )),
                                                Content::Error(error) => {
                                                    Err(anyhow::anyhow!(error))
                                                }
                                            };
                                            if let Err(error) = result {
                                                ws.app.report_workspace_error(format!(
                                                    "Cannot open workspace: {error:#}"
                                                ));
                                            }
                                        }
                                        WorkspaceMessage::WorkspaceDestination { target } => {
                                            ws.app.save_workspace(Some(target))
                                        }
                                        WorkspaceMessage::RequestWorkspace => ws
                                            .app
                                            .persist_workspace(volna_core::Instant::now(), true),
                                    }
                                    ws.after(None, cx);
                                }
                                HostEvent::Theme(theme) => crate::theme::install(*theme, cx),
                                HostEvent::Settings(map) => {
                                    ws.app.set_host_settings(map);
                                    ws.after(None, cx);
                                }
                                HostEvent::Command(command) => ws.dispatch(command, None, cx),
                                HostEvent::DebugState => log::info!(
                                    "STATE {}\ntrace_loaded={}\n{}",
                                    ws.debug_state(),
                                    matches!(
                                        ws.app.trace_state(),
                                        volna_core::document::TraceState::Loaded(_)
                                    ),
                                    ws.app.frames.debug_state()
                                ),
                            });
                        }
                    })
                    .detach();
                    if let Ok(f) =
                        js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("volnaReady"))
                        && let Some(f) = f.dyn_ref::<js_sys::Function>()
                    {
                        f.call0(&js_sys::global()).ok();
                    }
                }
                Err(e) => log::error!("failed to open window: {e:#}"),
            }
        });
        APP.with(|slot| *slot.borrow_mut() = Some(handle));
        Ok(())
    }

    thread_local! {
        static APP: RefCell<Option<gpui_kit::ApplicationHandle>> = const { RefCell::new(None) };
    }
}

#[cfg(test)]
mod tests {
    use super::window_decorations;
    use gpui_kit::WindowDecorations;

    #[test]
    fn client_decorations_unless_server_is_requested() {
        assert_eq!(window_decorations(None), WindowDecorations::Client);
        assert_eq!(window_decorations(Some("")), WindowDecorations::Client);
        assert_eq!(
            window_decorations(Some("client")),
            WindowDecorations::Client
        );
        assert_eq!(
            window_decorations(Some(" server ")),
            WindowDecorations::Server
        );
        assert_eq!(window_decorations(Some("nope")), WindowDecorations::Client);
    }
}
