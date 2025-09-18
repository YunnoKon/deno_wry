use serde::{ Deserialize, Serialize };
use std::{
    collections::HashMap, 
    io::{ self, Read, Write }, 
    thread,
    fs, 
    borrow::Cow
};
use byteorder::{ WriteBytesExt, ReadBytesExt, LittleEndian };
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ ActiveEventLoop, EventLoop, EventLoopProxy },
    window::{Window, WindowAttributes, WindowId},
    dpi::{LogicalSize}
};
use wry::{
    http::Response, WebView, WebViewBuilder, WebViewBuilderExtWindows
};

#[derive(Debug, Deserialize)]
struct WindowOptions {
    width: u32,
    height: u32,
    resizable: bool,
    maximized: bool,
    title: String,
    preload: Option<String>
}

#[derive(Debug, Deserialize)]
#[serde(tag="method")]
enum Request {
    CreateWindow { id: u32, options: WindowOptions },
    LoadUrl { id: u32, url: String },
    LoadHtml { id: u32, html: String },
    EmitToWebview { id: u32, channel: String, payload: String },
    Exit
}

#[derive(Debug, Serialize)]
#[serde(tag="event")]
enum EventOut {
    Init,
    WindowCreated { id: u32 },
    WindowClosed { id: u32 },
    ResponseOk { id: u32 },
    IPCMessage { id: u32, body: String }
}

fn send_event(ev: &EventOut) {
    let buf: Vec<u8> = rmp_serde::to_vec_named(ev).unwrap();
    let mut stdout = io::stdout();
    stdout.write_u32::<LittleEndian>(buf.len() as u32).unwrap();
    stdout.write_all(&buf).unwrap();
    stdout.flush().unwrap();
}

fn mime_type(path: &str) -> &'static str {
    match path.split('.').last().unwrap_or("") {
        "html" => "text/html",
        "css"  => "text/css",
        "js"   => "application/javascript",
        "png"  => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg"  => "image/svg+xml",
        _ => "text/plain",
    }
}
struct App {
    proxy: EventLoopProxy<Request>,
    windows: HashMap<u32, (Window, WebView)>,
    window_id_map: HashMap<WindowId, u32>
}

impl ApplicationHandler<Request> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop){
        send_event(&EventOut::Init);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, req: Request) {
        match req {
            Request::CreateWindow { id, options } => {
                let attrs: WindowAttributes = Window::default_attributes()
                .with_inner_size(LogicalSize::new(options.width, options.height))
                .with_title(options.title)
                .with_resizable(options.resizable)
                .with_maximized(options.maximized);
                
                let window: Window = event_loop.create_window(attrs).unwrap();

                let mut builder: WebViewBuilder = wry::WebViewBuilder::new()
                .with_https_scheme(true)
                .with_custom_protocol("app".to_string(), move |_id, request| {
                    let mut path = request.uri().path().to_string();

                    #[cfg(windows)]
                    // Handle window path like "/C:/hello/index.html" → "C:/hello/index.html"
                    if path.starts_with('/') && path.chars().nth(2) == Some(':') {
                        path.remove(0);
                    }

                    match fs::read(&path) {
                        Ok(contents) => {
                            Response::builder()
                                .header("Content-Type", mime_type(&path))
                                .body(Cow::Owned(contents))
                                .expect("Failed to build response")
                        }
                        Err(_) => {
                            Response::builder()
                                .status(404)
                                .body(Cow::Borrowed("Not Found".as_bytes()))
                                .expect("Failed to build 404 response")
                        }
                    }
                });

                if let Some(script_path) = options.preload {
                    let default_script = include_str!("preload.js");
                    let user_script: String = fs::read_to_string(script_path).unwrap();
                    let script = format!("{}\n{}", default_script, user_script);
                    builder = builder.with_initialization_script(&script);
                }

                builder = builder.with_ipc_handler(move |msg| {
                    let window_id = id;
                    send_event(&EventOut::IPCMessage {
                        id: window_id,
                        body: msg.body().to_string() 
                    });
                });

                let webview: WebView = builder.build(&window).unwrap();

                self.window_id_map.insert(window.id(), id);
                self.windows.insert(id, (window, webview));

                send_event(&EventOut::WindowCreated { id });
                send_event(&EventOut::ResponseOk { id });
            }
            Request::LoadUrl { id, url } => {
                if let Some((_, webview)) = self.windows.get_mut(&id){
                    webview.load_url(&url).unwrap();
                    send_event(&EventOut::ResponseOk { id });
                }
            }
            Request::LoadHtml { id, html } => {
                if let Some((_, webview)) = self.windows.get_mut(&id){
                    webview.load_html(&html).unwrap();
                    send_event(&EventOut::ResponseOk { id });
                }
            }
            Request::Exit => {
                event_loop.exit();
            }
            Request::EmitToWebview { id, channel, payload } => {
                if let Some((_, webview)) = self.windows.get_mut(&id){
                    let script = format!(
                        "window.__denoEmit({{ channel: {:?}, payload: {:?} }});",
                        channel,
                        payload
                    );
                    webview.evaluate_script(&script).unwrap();
                }
            }
        }
    }

    fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            window_id: winit::window::WindowId,
            event: WindowEvent,
        ) {
        if let WindowEvent::CloseRequested = event {
            // searching hashmap for window id and remove it
            if let Some(id) = self.window_id_map.remove(&window_id){
                self.windows.remove(&id);
                send_event(&EventOut::WindowClosed { id });
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop: EventLoop<Request> = EventLoop::<Request>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    let proxy_clone = proxy.clone();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut locked = stdin.lock();
        loop {
            let len = match locked.read_u32::<LittleEndian>() {
                Ok(l) => l as usize,
                Err(_) => break,
            };

            let mut buf = vec![0u8; len];
            if let Err(_) = locked.read_exact(&mut buf) {
                break;
            }

            if let Ok(req) = rmp_serde::from_slice::<Request>(&buf) {
                let _ = proxy_clone.send_event(req);
            }
        }
    });

    let mut app = App {
        proxy,
        windows: HashMap::new(),
        window_id_map: HashMap::new()
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}