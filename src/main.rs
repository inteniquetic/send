use eframe::{Result as EframeResult, egui};
use egui::{Color32, RichText, ScrollArea, TextEdit, Ui};
use egui_extras::syntax_highlighting::{CodeTheme, highlight};
use egui_extras::{Size, StripBuilder, TableBuilder};
use reqwest::{Method, header::HeaderMap};
use rfd;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Instant;
use tokio::runtime::Runtime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HttpRequest {
    id: String,
    name: String,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    body_type: BodyType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum BodyType {
    None,
    Raw,
    Json,
    FormData,
}

#[derive(Debug, Clone)]
struct HttpResponse {
    status: u16,
    status_text: String,
    headers: HashMap<String, String>,
    body: String,
    time: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Collection {
    id: String,
    name: String,
    requests: Vec<HttpRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Environment {
    name: String,
    variables: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppStorage {
    collections: Vec<Collection>,
    environments: Vec<Environment>,
}

struct SendApp {
    // Collections
    collections: Vec<Collection>,
    selected_collection: Option<usize>,
    selected_request: Option<usize>,
    // Current request
    current_request: HttpRequest,
    // Response
    current_response: Option<HttpResponse>,
    is_loading: bool,
    // Environment
    environments: Vec<Environment>,
    selected_environment: Option<usize>,
    // UI State
    show_collections: bool,
    show_environment: bool,
    response_tab: ResponseTab,
    // Runtime for async operations
    runtime: Runtime,
    response_receiver: Option<mpsc::Receiver<Result<HttpResponse, String>>>,
    // Dialogs
    new_collection_dialog: bool,
    new_collection_name: String,
    new_request_dialog: bool,
    new_request_name: String,
    // Auto-save
    workspace_file: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
enum ResponseTab {
    Body,
    Headers,
    Cookies,
}

impl Default for SendApp {
    fn default() -> Self {
        Self {
            collections: vec![Collection {
                id: Uuid::new_v4().to_string(),
                name: "Default Collection".to_string(),
                requests: vec![],
            }],
            selected_collection: Some(0),
            selected_request: None,
            current_request: HttpRequest {
                id: Uuid::new_v4().to_string(),
                name: "New Request".to_string(),
                method: "GET".to_string(),
                url: "https://httpbin.org/get".to_string(),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: String::new(),
                body_type: BodyType::None,
            },
            current_response: None,
            is_loading: false,
            environments: vec![Environment {
                name: "Default".to_string(),
                variables: HashMap::new(),
            }],
            selected_environment: Some(0),
            show_collections: true,
            show_environment: false,
            response_tab: ResponseTab::Body,
            runtime: Runtime::new().unwrap(),
            response_receiver: None,
            new_collection_dialog: false,
            new_collection_name: String::new(),
            new_request_dialog: false,
            new_request_name: String::new(),
            workspace_file: None,
        }
    }
}

impl eframe::App for SendApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for response
        if let Some(receiver) = &self.response_receiver {
            if let Ok(result) = receiver.try_recv() {
                match result {
                    Ok(response) => {
                        self.current_response = Some(response);
                        self.is_loading = false;
                    }
                    Err(error) => {
                        self.current_response = Some(HttpResponse {
                            status: 0,
                            status_text: "Error".to_string(),
                            headers: HashMap::new(),
                            body: error,
                            time: 0,
                        });
                        self.is_loading = false;
                    }
                }
                self.response_receiver = None;
            }
        }

        // Top panel
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Collection").clicked() {
                        self.new_collection_dialog = true;
                        ui.close_menu();
                    }
                    if ui.button("New Request").clicked() {
                        self.new_request_dialog = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save Workspace...").clicked() {
                        self.save_to_file();
                        ui.close_menu();
                    }
                    if ui.button("Load Workspace...").clicked() {
                        self.load_from_file();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export Collection...").clicked() {
                        self.export_collection();
                        ui.close_menu();
                    }
                    if ui.button("Import Collection...").clicked() {
                        self.import_collection();
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_collections, "Collections");
                    ui.checkbox(&mut self.show_environment, "Environment");
                });
            });
        });

        // Left panel - Collections
        if self.show_collections {
            egui::SidePanel::left("collections_panel")
                .min_width(250.0)
                .show(ctx, |ui| {
                    self.draw_collections_panel(ui);
                });
        }

        // Right panel - Environment
        if self.show_environment {
            egui::SidePanel::right("environment_panel")
                .min_width(250.0)
                .show(ctx, |ui| {
                    self.draw_environment_panel(ui);
                });
        }

        // Central panel
        egui::CentralPanel::default().show(ctx, |ui| {
            StripBuilder::new(ui)
                .size(Size::remainder().at_least(300.0))
                .size(Size::remainder().at_least(200.0))
                .vertical(|mut strip| {
                    strip.cell(|ui| {
                        self.draw_request_panel(ui);
                    });
                    strip.cell(|ui| {
                        self.draw_response_panel(ui);
                    });
                });
        });

        // Dialogs
        self.draw_dialogs(ctx);
    }
}

impl SendApp {
    fn save_current_request(&mut self) {
        if let (Some(collection_idx), Some(request_idx)) = (self.selected_collection, self.selected_request) {
            if collection_idx < self.collections.len() && request_idx < self.collections[collection_idx].requests.len() {
                self.collections[collection_idx].requests[request_idx] = self.current_request.clone();
                self.auto_save_workspace();
            }
        }
    }

    fn auto_save_workspace(&self) {
        if let Some(path) = &self.workspace_file {
            let data = AppStorage {
                collections: self.collections.clone(),
                environments: self.environments.clone(),
            };
            if let Ok(json) = serde_json::to_string_pretty(&data) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    fn resolve_value(&self, input: &str) -> String {
        let mut result = input.to_string();
        if let Some(env_idx) = self.selected_environment {
            if env_idx < self.environments.len() {
                let env = &self.environments[env_idx];
                for (key, value) in &env.variables {
                    let placeholder = format!("{{{{{}}}}}", key);
                    result = result.replace(&placeholder, value);
                }
            }
        }
        result
    }

    fn save_to_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Save Workspace")
            .add_filter("JSON", &["json"])
            .save_file()
        {
            let data = AppStorage {
                collections: self.collections.clone(),
                environments: self.environments.clone(),
            };
            let json = serde_json::to_string_pretty(&data).unwrap();
            if std::fs::write(&path, json).is_ok() {
                self.workspace_file = Some(path);
            }
        }
    }

    fn load_from_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Load Workspace")
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(storage) = serde_json::from_str::<AppStorage>(&content) {
                    self.collections = storage.collections;
                    self.environments = storage.environments;
                    self.selected_collection = self.collections.get(0).map(|_| 0);
                    self.selected_request = None;
                    self.selected_environment = self.environments.get(0).map(|_| 0);
                    self.workspace_file = Some(path);
                }
            }
        }
    }

    fn export_collection(&self) {
        if let Some(idx) = self.selected_collection {
            if let Some(collection) = self.collections.get(idx) {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(&format!("Export '{}'", collection.name))
                    .add_filter("JSON", &["json"])
                    .save_file()
                {
                    let json = serde_json::to_string_pretty(collection).unwrap();
                    std::fs::write(path, json).ok();
                }
            }
        }
    }

    fn import_collection(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import Collection")
            .add_filter("JSON", &["json"])
            .pick_file()
        {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(collection) = serde_json::from_str::<Collection>(&content) {
                    self.collections.push(collection);
                }
            }
        }
    }

    fn draw_collections_panel(&mut self, ui: &mut Ui) {
        ui.heading("Collections");
        ui.separator();
        ScrollArea::vertical().show(ui, |ui| {
            for (collection_idx, collection) in self.collections.iter_mut().enumerate() {
                let selected = self.selected_collection == Some(collection_idx);
                let response = ui.selectable_label(selected, &collection.name);
                if response.clicked() {
                    self.selected_collection = Some(collection_idx);
                    self.selected_request = None;
                }
                if selected {
                    ui.indent("requests", |ui| {
                        for (request_idx, request) in collection.requests.iter().enumerate() {
                            let selected_req = self.selected_request == Some(request_idx);
                            let method_color = match request.method.as_str() {
                                "GET" => Color32::from_rgb(0, 128, 0),
                                "POST" => Color32::from_rgb(255, 165, 0),
                                "PUT" => Color32::from_rgb(0, 0, 255),
                                "DELETE" => Color32::from_rgb(255, 0, 0),
                                _ => Color32::GRAY,
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&request.method).color(method_color));
                                if ui.selectable_label(selected_req, &request.name).clicked() {
                                    self.selected_request = Some(request_idx);
                                    self.current_request = request.clone();
                                }
                            });
                        }
                    });
                }
            }
        });
    }

    fn draw_environment_panel(&mut self, ui: &mut Ui) {
        ui.heading("Environment");
        ui.separator();
        // Environment selector
        if let Some(env_idx) = self.selected_environment {
            if env_idx < self.environments.len() {
                egui::ComboBox::from_label("Environment")
                    .selected_text(&self.environments[env_idx].name)
                    .show_ui(ui, |ui| {
                        for (idx, env) in self.environments.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.selected_environment,
                                Some(idx),
                                &env.name,
                            );
                        }
                    });
            }
        }
        ui.separator();
        // Variables
        if let Some(env_idx) = self.selected_environment {
            if env_idx < self.environments.len() {
                ui.label("Variables:");
                ScrollArea::vertical().show(ui, |ui| {
                    let env = &mut self.environments[env_idx];
                    let mut to_remove = Vec::new();
                    let mut env_changed = false;
                    for (i, (key, value)) in env.variables.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}:", key));
                            if ui.text_edit_singleline(value).changed() {
                                env_changed = true;
                            }
                            if ui.button("🗑").clicked() {
                                to_remove.push(i);
                            }
                        });
                    }
                    if !to_remove.is_empty() {
                        for &i in to_remove.iter().rev() {
                            let keys: Vec<String> = env.variables.keys().cloned().collect();
                            if i < keys.len() {
                                env.variables.remove(&keys[i]);
                            }
                        }
                        env_changed = true;
                    }
                    if ui.button("Add Variable").clicked() {
                        env.variables
                            .insert(format!("key{}", env.variables.len()), "value".to_string());
                        env_changed = true;
                    }
                    if env_changed {
                        self.auto_save_workspace();
                    }
                });
            }
        }
    }

    fn draw_request_panel(&mut self, ui: &mut Ui) {
        ui.heading("Request");
        ui.separator();
        // Method and URL
        ui.horizontal(|ui| {
            let method_response = egui::ComboBox::from_id_source("method")
                .selected_text(&self.current_request.method)
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.current_request.method, "GET".to_string(), "GET");
                    ui.selectable_value(
                        &mut self.current_request.method,
                        "POST".to_string(),
                        "POST",
                    );
                    ui.selectable_value(&mut self.current_request.method, "PUT".to_string(), "PUT");
                    ui.selectable_value(
                        &mut self.current_request.method,
                        "DELETE".to_string(),
                        "DELETE",
                    );
                    ui.selectable_value(
                        &mut self.current_request.method,
                        "PATCH".to_string(),
                        "PATCH",
                    );
                    ui.selectable_value(
                        &mut self.current_request.method,
                        "HEAD".to_string(),
                        "HEAD",
                    );
                    ui.selectable_value(
                        &mut self.current_request.method,
                        "OPTIONS".to_string(),
                        "OPTIONS",
                    );
                });
            if method_response.response.changed() {
                self.save_current_request();
            }
            let url_response = ui.add(
                TextEdit::singleline(&mut self.current_request.url)
                    .hint_text("Enter URL (supports {{variable}})...")
                    .desired_width(ui.available_width() - 80.0),
            );
            if url_response.changed() {
                self.save_current_request();
            }
            if ui
                .button(if self.is_loading { "⏸" } else { "Send" })
                .clicked()
                && !self.is_loading
            {
                self.send_request();
            }
        });
        ui.separator();
        // Tabs for request details
        ui.horizontal(|ui| {
            if ui.selectable_value(&mut self.current_request.body_type, BodyType::None, "None").changed() {
                self.save_current_request();
            }
            if ui.selectable_value(&mut self.current_request.body_type, BodyType::Raw, "Raw").changed() {
                self.save_current_request();
            }
            if ui.selectable_value(&mut self.current_request.body_type, BodyType::Json, "JSON").changed() {
                self.save_current_request();
            }
            if ui.selectable_value(
                &mut self.current_request.body_type,
                BodyType::FormData,
                "Form Data",
            ).changed() {
                self.save_current_request();
            }
        });
        ui.separator();
        // Headers
        ui.collapsing("Headers", |ui| {
            let mut to_remove = Vec::new();
            let mut headers_changed = false;
            for (i, (key, value)) in self.current_request.headers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let key_response = ui.add(TextEdit::singleline(key).hint_text("Header name"));
                    let value_response = ui.add(
                        TextEdit::singleline(value)
                            .hint_text("Header value (supports {{variable}})"),
                    );
                    if key_response.changed() || value_response.changed() {
                        headers_changed = true;
                    }
                    if ui.button("🗑").clicked() {
                        to_remove.push(i);
                    }
                });
            }
            for &i in to_remove.iter().rev() {
                self.current_request.headers.remove(i);
                headers_changed = true;
            }
            if ui.button("Add Header").clicked() {
                self.current_request
                    .headers
                    .push((String::new(), String::new()));
                headers_changed = true;
            }
            if headers_changed {
                self.save_current_request();
            }
        });

        // Body
        match self.current_request.body_type {
            BodyType::None => {}
            BodyType::Raw | BodyType::FormData => {
                ui.label("Body:");
                let body_response = ui.add(
                    TextEdit::multiline(&mut self.current_request.body)
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .hint_text("Enter raw or form data..."),
                );
                if body_response.changed() {
                    self.save_current_request();
                }
            }
            BodyType::Json => {
                ui.label(RichText::new("Body (JSON)").color(Color32::BLUE));

                let mut code = self.current_request.body.clone();

                let theme = CodeTheme::default();
                let lang = "json";
                let job = highlight(ui.ctx(), ui.style(), &theme, &code, lang);

                let json_response = ui.add(
                    TextEdit::multiline(&mut code)
                        .code_editor()
                        .desired_rows(10)
                        .desired_width(ui.available_width()),
                );

                if code != self.current_request.body {
                    self.current_request.body = code;
                    if json_response.changed() {
                        self.save_current_request();
                    }
                }
            }
        }
    }

    fn draw_response_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("Response");
            if self.is_loading {
                ui.spinner();
            }
        });
        ui.separator();
        if let Some(response) = &self.current_response {
            // Status and time
            ui.horizontal(|ui| {
                let status_color = if response.status >= 200 && response.status < 300 {
                    Color32::from_rgb(0, 128, 0)
                } else if response.status >= 400 {
                    Color32::from_rgb(255, 0, 0)
                } else {
                    Color32::from_rgb(255, 165, 0)
                };
                ui.label(
                    RichText::new(format!(
                        "Status: {} {}",
                        response.status, response.status_text
                    ))
                    .color(status_color),
                );
                ui.label(format!("Time: {}ms", response.time));
            });
            ui.separator();
            // Response tabs
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.response_tab, ResponseTab::Body, "Body");
                ui.selectable_value(&mut self.response_tab, ResponseTab::Headers, "Headers");
                ui.selectable_value(&mut self.response_tab, ResponseTab::Cookies, "Cookies");
            });
            ui.separator();
            // Response content
            ScrollArea::vertical().show(ui, |ui| match self.response_tab {
                ResponseTab::Body => {
                    ui.add(
                        TextEdit::multiline(&mut response.body.clone())
                            .desired_rows(15)
                            .desired_width(ui.available_width())
                            .interactive(false),
                    );
                }
                ResponseTab::Headers => {
                    for (key, value) in &response.headers {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(key).strong());
                            ui.label(value);
                        });
                    }
                }
                ResponseTab::Cookies => {
                    ui.label("Cookie support coming soon...");
                }
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("No response yet. Send a request to see the response here.");
            });
        }
    }

    fn draw_dialogs(&mut self, ctx: &egui::Context) {
        // New Collection Dialog
        if self.new_collection_dialog {
            egui::Window::new("New Collection")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Collection Name:");
                    ui.text_edit_singleline(&mut self.new_collection_name);
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            if !self.new_collection_name.trim().is_empty() {
                                self.collections.push(Collection {
                                    id: Uuid::new_v4().to_string(),
                                    name: self.new_collection_name.clone(),
                                    requests: vec![],
                                });
                                self.new_collection_name.clear();
                                self.new_collection_dialog = false;
                                self.auto_save_workspace();
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.new_collection_name.clear();
                            self.new_collection_dialog = false;
                        }
                    });
                });
        }

        // New Request Dialog
        if self.new_request_dialog {
            egui::Window::new("New Request")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Request Name:");
                    ui.text_edit_singleline(&mut self.new_request_name);
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            if !self.new_request_name.trim().is_empty() {
                                if let Some(collection_idx) = self.selected_collection {
                                    if collection_idx < self.collections.len() {
                                        let mut new_request = self.current_request.clone();
                                        new_request.id = Uuid::new_v4().to_string();
                                        new_request.name = self.new_request_name.clone();
                                        self.collections[collection_idx].requests.push(new_request);
                                        self.new_request_name.clear();
                                        self.new_request_dialog = false;
                                        self.auto_save_workspace();
                                    }
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.new_request_name.clear();
                            self.new_request_dialog = false;
                        }
                    });
                });
        }
    }

    fn send_request(&mut self) {
        self.is_loading = true;
        self.current_response = None;
        let request = self.current_request.clone();
        let (tx, rx) = mpsc::channel();
        self.response_receiver = Some(rx);

        let resolved_url = self.resolve_value(&request.url);
        let mut resolved_headers = Vec::new();
        for (k, v) in &request.headers {
            resolved_headers.push((k.clone(), self.resolve_value(v)));
        }
        let resolved_body = self.resolve_value(&request.body);

        self.runtime.spawn(async move {
            let start_time = Instant::now();
            let method = match request.method.as_str() {
                "GET" => Method::GET,
                "POST" => Method::POST,
                "PUT" => Method::PUT,
                "DELETE" => Method::DELETE,
                "PATCH" => Method::PATCH,
                "HEAD" => Method::HEAD,
                "OPTIONS" => Method::OPTIONS,
                _ => Method::GET,
            };

            let client = reqwest::Client::new();
            let mut req_builder = client.request(method, &resolved_url);

            for (key, value) in &resolved_headers {
                if !key.trim().is_empty() && !value.trim().is_empty() {
                    req_builder = req_builder.header(key, value);
                }
            }

            if !resolved_body.trim().is_empty() {
                req_builder = req_builder.body(resolved_body);
            }

            let result = match req_builder.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let status_text = response
                        .status()
                        .canonical_reason()
                        .unwrap_or("Unknown")
                        .to_string();
                    let mut headers = HashMap::new();
                    for (key, value) in response.headers() {
                        headers.insert(key.to_string(), value.to_str().unwrap_or("").to_string());
                    }
                    let body = response
                        .text()
                        .await
                        .unwrap_or_else(|e| format!("Error reading body: {}", e));
                    let time = start_time.elapsed().as_millis();

                    Ok(HttpResponse {
                        status,
                        status_text,
                        headers,
                        body,
                        time,
                    })
                }
                Err(e) => Err(format!("Request failed: {}", e)),
            };

            let _ = tx.send(result);
        });
    }
}

fn main() -> EframeResult<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Send - HTTP Client",
        options,
        Box::new(|_cc| Ok(Box::new(SendApp::default()))),
    )
}
