//! Hosted mode: GridCraft running in a frame of another app (`?host` in the URL), such as a notes
//! app that edits workbooks with it. The host hands workbooks in and takes saves out with
//! `postMessage`, in place of the browser's file pickers and downloads.
//!
//! Every workbook the host opens carries a token (`doc`) that GridCraft repeats on everything it
//! says about that workbook, so a late message about an earlier one is never taken for the
//! current one. GridCraft shows one workbook at a time when hosted: an open replaces the others.
//!
//! From the host (only the frame's own parent window is listened to):
//! - `{type: "open", doc, name, bytes: Uint8Array}` opens a workbook (as Open would).
//! - `{type: "command", doc, id}` runs one of [`crate::host_rules::HOST_COMMANDS`] on that workbook.
//! - `{type: "theme", value: "light" | "dark" | "system"}` sets the appearance, to match the host's.
//! - `{type: "page", dark: bool}` is ignored: GridCraft has no dark page view (a sheet has no page).
//!
//! To the host:
//! - `{type: "ready"}` once the app is running and listening.
//! - `{type: "opened", doc, ok, ms, error?}` after an open.
//! - `{type: "drawn", doc, ms, frames, frameMs}` once the first frame after an open has been drawn.
//! - `{type: "save", doc, name, bytes: Uint8Array}` for every file GridCraft would otherwise
//!   download: Save, Save As and the exports (CSV, PDF, ...). Saving an `.xlsx` counts as saving
//!   the workbook; the host tells the two apart by the name.
//! - `{type: "dirty", doc, dirty: bool}` when unsaved changes appear or go away.
//!
//! Messages are acted on when they arrive, not on the next frame: a hidden page draws no frames,
//! and a host asking for a save as it hides or quits must still get one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gridcraft_ui_egui::SheetApp;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;

use crate::host_rules::{MAX_OPEN_BYTES, clean_doc, clean_name, host_may_run, is_workbook};

/// What the page knows about its host: the workbook it is showing, the files handed over since
/// they were last settled, and the unsaved-changes state last reported (so it is said once per
/// change, not every frame).
#[derive(Default)]
pub struct HostState {
    doc: RefCell<String>,
    saved: RefCell<Vec<String>>,
    last_dirty: Cell<Option<bool>>,
    drawing: RefCell<Option<Drawing>>,
}

struct Drawing {
    doc: String,
    since: f64,
}

pub fn after_frame(state: &HostState, frame_ms: f64) {
    let Some(d) = state.drawing.borrow_mut().take() else { return };
    post(&[
        ("type", "drawn".into()),
        ("doc", d.doc.into()),
        ("ms", (js_sys::Date::now() - d.since).into()),
        ("frames", 1u32.into()),
        ("frameMs", frame_ms.into()),
    ]);
}

/// Whether this page runs hosted: `host` is a key of the URL's query, and the page is in a frame.
pub fn hosted() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let asked = window.location().search().ok().and_then(|q| web_sys::UrlSearchParams::new_with_str(&q).ok()).is_some_and(|p| p.has("host"));
    asked && parent(&window).is_some()
}

/// The host window, when this page is in a frame.
fn parent(window: &web_sys::Window) -> Option<web_sys::Window> {
    let parent = window.parent().ok().flatten()?;
    if js_sys::Object::is(parent.as_ref(), window.as_ref()) { None } else { Some(parent) }
}

/// Sends a message to the host. A page that is not hosted sends nothing.
pub fn post(fields: &[(&str, JsValue)]) {
    let Some(window) = web_sys::window() else { return };
    let Some(host) = parent(&window) else { return };
    let msg = js_sys::Object::new();
    for (k, v) in fields {
        let _ = js_sys::Reflect::set(&msg, &JsValue::from_str(k), v);
    }
    // The host page may be a file: page, whose origin can't be named here; it only ever gets its own
    // workbook back.
    if let Err(e) = host.post_message(&msg, "*") {
        log::error!("couldn't reach the host: {e:?}");
    }
}

/// Hands a saved file to the host (`download` in hosted mode), tagged with its workbook.
pub fn save(state: &HostState, path: &str, bytes: &[u8]) {
    let name = clean_name(path).unwrap_or_else(|| "Book.xlsx".into());
    let doc = state.doc.borrow().clone();
    post(&[("type", "save".into()), ("doc", doc.into()), ("name", name.as_str().into()), ("bytes", js_sys::Uint8Array::from(bytes).into())]);
    state.saved.borrow_mut().push(name);
}

/// After a frame or a message: a workbook handed to the host counts as saved under the name it was
/// saved as, and the host hears when unsaved changes appear or go away (once per change, and only
/// about a workbook of its own).
pub fn sync(state: &HostState, app: &mut SheetApp) {
    let saved = std::mem::take(&mut *state.saved.borrow_mut());
    if let Some(name) = saved.iter().rev().find(|name| is_workbook(name))
        && let Some(doc) = app.session.active_mut()
    {
        doc.saved = doc.wb.clone();
        if doc.path.is_none() {
            doc.title = name.clone();
        }
    }
    let doc = state.doc.borrow().clone();
    if doc.is_empty() {
        return;
    }
    let dirty = app.session.documents().iter().any(|d| d.is_dirty());
    if state.last_dirty.get() != Some(dirty) {
        state.last_dirty.set(Some(dirty));
        post(&[("type", "dirty".into()), ("doc", doc.into()), ("dirty", dirty.into())]);
    }
}

/// Opens the host's workbook in place of whatever is open. Nothing asks about unsaved changes: the
/// host decides, and a "Save changes?" box must not take the place of the open.
fn open(app: &mut SheetApp, name: &str, bytes: &[u8]) -> Result<(), String> {
    app.open_bytes(name, bytes)
        .map_err(|e| if e.contains(gridcraft_engine::io::PASSWORD_REQUIRED) { "the workbook is password-protected".to_string() } else { e })?;
    // The new workbook is the last one; the host's earlier workbook, and a blank one, go.
    while app.session.documents().len() > 1 {
        app.session.close_document(0);
    }
    Ok(())
}

/// Listens to the host and acts on its messages at once, on `app`.
pub fn listen(app: Rc<RefCell<SheetApp>>, state: Rc<HostState>, ctx: egui::Context) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let host = parent(&window).ok_or("not in a frame")?;
    let on_message = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
        // Only the frame's own host is listened to.
        let from_host = e.source().is_some_and(|s| js_sys::Object::is(s.as_ref(), host.as_ref()));
        if !from_host {
            return;
        }
        let data = e.data();
        let field = |k: &str| js_sys::Reflect::get(&data, &JsValue::from_str(k)).ok();
        let text = |k: &str| field(k).and_then(|v| v.as_string());
        let kind = text("type").unwrap_or_default();
        let doc = clean_doc(&text("doc").unwrap_or_default());
        // The page is between frames whenever a message arrives, so the app is free; if it is not
        // (a message during a save), the message is dropped and the host's timeout reports it.
        let Ok(mut app) = app.try_borrow_mut() else {
            log::warn!("host: busy, {kind} dropped");
            return;
        };
        match kind.as_str() {
            "open" => {
                let started = js_sys::Date::now();
                let opened = (|| {
                    let name = text("name").and_then(|n| clean_name(&n)).ok_or("the workbook has no name")?;
                    let bytes = field("bytes").and_then(|v| v.dyn_into::<js_sys::Uint8Array>().ok()).ok_or("the workbook has no bytes")?;
                    if bytes.length() > MAX_OPEN_BYTES {
                        return Err("the workbook is larger than 200 MB".to_string());
                    }
                    open(&mut app, &name, &bytes.to_vec())
                })();
                state.last_dirty.set(None);
                let ms = js_sys::Date::now() - started;
                match opened {
                    Ok(()) => {
                        *state.doc.borrow_mut() = doc.clone();
                        *state.drawing.borrow_mut() = Some(Drawing { doc: doc.clone(), since: started });
                        post(&[("type", "opened".into()), ("doc", doc.into()), ("ok", true.into()), ("ms", ms.into())]);
                    }
                    Err(err) => {
                        // Whatever is still on show is no workbook of the host's: its commands are ignored.
                        state.doc.borrow_mut().clear();
                        *state.drawing.borrow_mut() = None;
                        log::warn!("host: open failed: {err}");
                        post(&[("type", "opened".into()), ("doc", doc.into()), ("ok", false.into()), ("error", err.into())]);
                    }
                }
                sync(&state, &mut app);
                ctx.request_repaint();
            }
            "command" => {
                let Some(id) = text("id") else { return };
                if doc.is_empty() || doc != *state.doc.borrow() {
                    log::warn!("host: {id} for a workbook that is no longer open; ignored");
                } else if !host_may_run(&id) {
                    log::warn!("host: command {id} is not one a host may run");
                } else {
                    // A cell still being typed is part of what the host is asking to save.
                    if id == "file.save" {
                        app.commit_edit(0, 0, false, false);
                    }
                    if let Err(err) = app.execute(&id, serde_json::json!({})) {
                        log::error!("host command {id} failed: {err}");
                    }
                }
                sync(&state, &mut app);
                ctx.request_repaint();
            }
            "theme" => {
                let Some(value) = text("value") else { return };
                if let Err(err) = app.execute("view.theme", serde_json::json!({"mode": value})) {
                    log::warn!("host: theme {value:?}: {err}");
                }
                ctx.request_repaint();
            }
            "page" => log::info!("host: GridCraft has no dark page view; page message ignored"),
            other => log::warn!("host: unknown message {other:?}"),
        }
    });
    window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_message.forget();
    Ok(())
}
