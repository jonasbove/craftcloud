//! The browser transport: `fetch` on the page's own origin, with OxiCloud's session cookies and
//! the CSRF double-submit header its middleware checks on writes. Uploads use `XMLHttpRequest`
//! instead (fetch can't report upload progress), with the browser's `File` as the body, so a
//! file streams from disk without being copied into wasm memory.

use std::cell::RefCell;
use std::rc::Rc;

use craftcloud_core::http::{HttpRequest, HttpResponse};
use std::collections::HashMap;

use craftcloud_ui_egui::backend::{Backend, LocalFile, NetResult, WallClock};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

pub struct FetchBackend {
    ctx: egui::Context,
    done: Rc<RefCell<Vec<(u64, NetResult)>>>,
    /// Files picked or dropped, by key, until they are uploaded or let go.
    files: Rc<RefCell<HashMap<u64, web_sys::File>>>,
    picked: Rc<RefCell<Vec<LocalFile>>>,
    progress: Rc<RefCell<Vec<(u64, u64)>>>,
    next_file: Rc<RefCell<u64>>,
}

impl FetchBackend {
    pub fn new(ctx: egui::Context) -> Self {
        Self { ctx, done: Rc::default(), files: Rc::default(), picked: Rc::default(), progress: Rc::default(), next_file: Rc::new(RefCell::new(1)) }
    }

    fn keep(&self, file: web_sys::File) -> LocalFile {
        let mut next = self.next_file.borrow_mut();
        let key = *next;
        *next += 1;
        let local = LocalFile { key, name: file.name(), size: file.size() as u64, mime: file.type_() };
        self.files.borrow_mut().insert(key, file);
        local
    }
}

/// The id of the page's one file input (see [`Backend::pick_files`]).
pub const UPLOAD_INPUT_ID: &str = "craftcloud-upload-input";

impl FetchBackend {
    /// Makes the hidden file input, puts it in the page and listens for its `change`.
    fn make_upload_input(&self, document: &web_sys::Document) -> Option<web_sys::HtmlInputElement> {
        let input = document.create_element("input").ok()?.dyn_into::<web_sys::HtmlInputElement>().ok()?;
        input.set_type("file");
        input.set_multiple(true);
        input.set_id(UPLOAD_INPUT_ID);
        input.set_attribute("hidden", "").ok()?;
        input.set_attribute("aria-hidden", "true").ok()?;
        document.body()?.append_child(&input).ok()?;
        let (files, picked, next, ctx) = (Rc::clone(&self.files), Rc::clone(&self.picked), Rc::clone(&self.next_file), self.ctx.clone());
        let chooser = input.clone();
        let on_change = Closure::<dyn FnMut()>::new(move || {
            let Some(list) = chooser.files() else { return };
            for i in 0..list.length() {
                if let Some(file) = list.get(i) {
                    let mut n = next.borrow_mut();
                    let key = *n;
                    *n += 1;
                    picked.borrow_mut().push(LocalFile { key, name: file.name(), size: file.size() as u64, mime: file.type_() });
                    files.borrow_mut().insert(key, file);
                }
            }
            ctx.request_repaint();
        });
        input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        // The input lives as long as the page, and so does its one handler.
        on_change.forget();
        Some(input)
    }
}

impl Backend for FetchBackend {
    fn send(&mut self, id: u64, request: HttpRequest) {
        let done = Rc::clone(&self.done);
        let ctx = self.ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch(request).await.map_err(|e| describe(&e));
            done.borrow_mut().push((id, result));
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) -> Vec<(u64, NetResult)> {
        std::mem::take(&mut *self.done.borrow_mut())
    }

    fn open_url(&mut self, url: &str) {
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href(url);
        }
    }

    fn now(&self) -> f64 {
        web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now() / 1000.0)
    }

    fn download(&mut self, url: &str, name: &str) {
        // A same-origin link with `download` saves the answer under `name` instead of showing it.
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let Ok(a) = document.create_element("a") else { return };
        let Ok(a) = a.dyn_into::<web_sys::HtmlAnchorElement>() else { return };
        a.set_href(url);
        a.set_download(name);
        a.click();
    }

    fn open_tab(&mut self, url: &str) {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
    }

    /// Opens the file chooser through one hidden `<input type=file id=craftcloud-upload-input>`
    /// that stays in the page: its change handler is set once, and a test driver (CDP's
    /// `DOM.setFileInputFiles`) can find it by id and hand it files without a dialog.
    fn pick_files(&mut self) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let existing = document.get_element_by_id(UPLOAD_INPUT_ID).and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok());
        let Some(input) = existing.or_else(|| self.make_upload_input(&document)) else { return };
        // Choosing the same file again must fire `change` again.
        input.set_value("");
        input.click();
    }

    fn take_picked(&mut self) -> Vec<LocalFile> {
        std::mem::take(&mut *self.picked.borrow_mut())
    }

    fn accept_dropped(&mut self, dropped: &[egui::DroppedFileHandle]) -> Vec<LocalFile> {
        dropped.iter().filter_map(|d| d.web_file().cloned()).map(|f| self.keep(f)).collect()
    }

    fn send_file(&mut self, id: u64, request: HttpRequest, file: u64) {
        let Some(file) = self.files.borrow().get(&file).cloned() else {
            self.done.borrow_mut().push((id, Err("the file is gone; choose it again".into())));
            return;
        };
        if let Err(e) = upload(id, &request, &file, &self.done, &self.progress, &self.ctx) {
            self.done.borrow_mut().push((id, Err(describe(&e))));
        }
    }

    fn poll_progress(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut *self.progress.borrow_mut())
    }

    fn forget_file(&mut self, file: u64) {
        self.files.borrow_mut().remove(&file);
    }

    fn wall_clock(&self) -> Option<WallClock> {
        let now = js_sys::Date::new_0();
        // getTimezoneOffset is UTC minus local time.
        Some(WallClock { unix: now.get_time() / 1000.0, utc_offset_minutes: -(now.get_timezone_offset() as i32) })
    }
}

async fn fetch(request: HttpRequest) -> Result<HttpResponse, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let headers = web_sys::Headers::new()?;
    for (name, value) in &request.headers {
        headers.append(name, value)?;
    }
    if request.needs_csrf()
        && let Some(token) = csrf_cookie(&window)
    {
        headers.append("X-CSRF-Token", &token)?;
    }
    let init = web_sys::RequestInit::new();
    init.set_method(request.method.as_str());
    init.set_headers(&headers);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    if !request.body.is_empty() {
        let body = js_sys::Uint8Array::from(request.body.as_slice());
        init.set_body(&body);
    }
    let req = web_sys::Request::new_with_str_and_init(&request.url, &init)?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&req)).await?.dyn_into()?;
    let mut out = HttpResponse::new(resp.status(), Vec::new());
    for name in ["ETag", "Retry-After"] {
        if let Some(value) = resp.headers().get(name)? {
            out = out.with_header(name, &value);
        }
    }
    let buffer = JsFuture::from(resp.array_buffer()?).await?;
    out.body = js_sys::Uint8Array::new(&buffer).to_vec();
    Ok(out)
}

/// Sends `file` as the body of `request` with `XMLHttpRequest`, reporting progress.
fn upload(
    id: u64,
    request: &HttpRequest,
    file: &web_sys::File,
    done: &Rc<RefCell<Vec<(u64, NetResult)>>>,
    progress: &Rc<RefCell<Vec<(u64, u64)>>>,
    ctx: &egui::Context,
) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let xhr = web_sys::XmlHttpRequest::new()?;
    xhr.open(request.method.as_str(), &request.url)?;
    for (name, value) in &request.headers {
        xhr.set_request_header(name, value)?;
    }
    if request.needs_csrf()
        && let Some(token) = csrf_cookie(&window)
    {
        xhr.set_request_header("X-CSRF-Token", &token)?;
    }
    xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
    let (sent, repaint) = (Rc::clone(progress), ctx.clone());
    let on_progress = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |e: web_sys::ProgressEvent| {
        sent.borrow_mut().push((id, e.loaded() as u64));
        repaint.request_repaint();
    });
    xhr.upload()?.set_onprogress(Some(on_progress.as_ref().unchecked_ref()));
    let (finished, repaint, answer) = (Rc::clone(done), ctx.clone(), xhr.clone());
    let on_end = Closure::<dyn FnMut()>::new(move || {
        let result = if answer.status().unwrap_or(0) == 0 {
            Err("the upload didn't reach the server".to_owned())
        } else {
            let mut out = HttpResponse::new(answer.status().unwrap_or(0), Vec::new());
            if let Ok(Some(etag)) = answer.get_response_header("ETag") {
                out = out.with_header("ETag", &etag);
            }
            if let Ok(body) = answer.response() {
                out.body = js_sys::Uint8Array::new(&body).to_vec();
            }
            Ok(out)
        };
        finished.borrow_mut().push((id, result));
        repaint.request_repaint();
    });
    // `loadend` fires once, after success, error or abort.
    xhr.set_onloadend(Some(on_end.as_ref().unchecked_ref()));
    xhr.send_with_opt_blob(Some(file))?;
    on_progress.forget();
    on_end.forget();
    Ok(())
}

/// OxiCloud's `oxicloud_csrf` cookie (readable by design; the session cookies are HttpOnly).
fn csrf_cookie(window: &web_sys::Window) -> Option<String> {
    let cookies = window.document()?.dyn_into::<web_sys::HtmlDocument>().ok()?.cookie().ok()?;
    cookies.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == "oxicloud_csrf").map(|(_, v)| v.to_owned())
}

fn describe(e: &JsValue) -> String {
    e.as_string().or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_else(|| "the request failed".into())
}
