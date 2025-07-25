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
    form_data: Vec<FormDataEntry>,
    url_encoded_data: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum BodyType {
    None,
    Raw,
    Json,
    FormData,
    UrlEncoded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum FormDataEntry {
    Text { key: String, value: String },
    File { key: String, file_path: String, file_name: String },
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

#[derive(Debug, Clone)]
struct Workspace {
    name: String,
    file_path: Option<std::path::PathBuf>,
    collections: Vec<Collection>,
    environments: Vec<Environment>,
    selected_collection: Option<usize>,
    selected_request: Option<usize>,
    selected_environment: Option<usize>,
}

struct SendApp {
    // Workspaces
    workspaces: Vec<Workspace>,
    current_workspace: usize,
    // Current request
    current_request: HttpRequest,
    // Response
    current_response: Option<HttpResponse>,
    is_loading: bool,
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
    new_workspace_dialog: bool,
    new_workspace_name: String,
}

#[derive(Debug, Clone, PartialEq)]
enum ResponseTab {
    Body,
    Headers,
    Cookies,
}

impl Default for SendApp {
    fn default() -> Self {
        let default_workspace = Workspace {
            name: "Default Workspace".to_string(),
            file_path: None,
            collections: vec![Collection {
                id: Uuid::new_v4().to_string(),
                name: "Default Collection".to_string(),
                requests: vec![],
            }],
            environments: vec![Environment {
                name: "Default".to_string(),
                variables: HashMap::new(),
            }],
            selected_collection: Some(0),
            selected_request: None,
            selected_environment: Some(0),
        };

        Self {
            workspaces: vec![default_workspace],
            current_workspace: 0,
            current_request: HttpRequest {
                id: Uuid::new_v4().to_string(),
                name: "New Request".to_string(),
                method: "GET".to_string(),
                url: "https://httpbin.org/get".to_string(),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: String::new(),
                body_type: BodyType::None,
                form_data: vec![],
                url_encoded_data: vec![],
            },
            current_response: None,
            is_loading: false,
            show_collections: true,
            show_environment: false,
            response_tab: ResponseTab::Body,
            runtime: Runtime::new().unwrap(),
            response_receiver: None,
            new_collection_dialog: false,
            new_collection_name: String::new(),
            new_request_dialog: false,
            new_request_name: String::new(),
            new_workspace_dialog: false,
            new_workspace_name: String::new(),
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
                    if ui.button("New Workspace").clicked() {
                        self.new_workspace_dialog = true;
                        ui.close_menu();
                    }
                    ui.separator();
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
                
                ui.separator();
                
                // Workspace tabs
                ui.horizontal(|ui| {
                    ui.label("Workspaces:");
                    for (idx, workspace) in self.workspaces.iter().enumerate() {
                        let selected = idx == self.current_workspace;
                        if ui.selectable_label(selected, &workspace.name).clicked() {
                            self.current_workspace = idx;
                        }
                    }
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
    fn current_workspace(&self) -> &Workspace {
        &self.workspaces[self.current_workspace]
    }

    fn current_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.current_workspace]
    }

    fn save_current_request(&mut self) {
        let current_request = self.current_request.clone();
        let workspace = self.current_workspace_mut();
        if let (Some(collection_idx), Some(request_idx)) = (workspace.selected_collection, workspace.selected_request) {
            if collection_idx < workspace.collections.len() && request_idx < workspace.collections[collection_idx].requests.len() {
                workspace.collections[collection_idx].requests[request_idx] = current_request;
                self.auto_save_workspace();
            }
        }
    }

    fn auto_save_workspace(&self) {
        let workspace = self.current_workspace();
        if let Some(path) = &workspace.file_path {
            let data = AppStorage {
                collections: workspace.collections.clone(),
                environments: workspace.environments.clone(),
            };
            if let Ok(json) = serde_json::to_string_pretty(&data) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    fn resolve_value(&self, input: &str) -> String {
        let mut result = input.to_string();
        let workspace = self.current_workspace();
        if let Some(env_idx) = workspace.selected_environment {
            if env_idx < workspace.environments.len() {
                let env = &workspace.environments[env_idx];
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
            let workspace = self.current_workspace_mut();
            let data = AppStorage {
                collections: workspace.collections.clone(),
                environments: workspace.environments.clone(),
            };
            let json = serde_json::to_string_pretty(&data).unwrap();
            if std::fs::write(&path, json).is_ok() {
                workspace.file_path = Some(path);
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
                    let workspace_name = path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Loaded Workspace")
                        .to_string();
                    
                    let selected_collection = if !storage.collections.is_empty() { Some(0) } else { None };
                    let selected_environment = if !storage.environments.is_empty() { Some(0) } else { None };
                    
                    let new_workspace = Workspace {
                        name: workspace_name,
                        file_path: Some(path),
                        collections: storage.collections,
                        environments: storage.environments,
                        selected_collection,
                        selected_request: None,
                        selected_environment,
                    };
                    
                    self.workspaces.push(new_workspace);
                    self.current_workspace = self.workspaces.len() - 1;
                }
            }
        }
    }

    fn export_collection(&self) {
        let workspace = self.current_workspace();
        if let Some(idx) = workspace.selected_collection {
            if let Some(collection) = workspace.collections.get(idx) {
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
                    self.current_workspace_mut().collections.push(collection);
                    self.auto_save_workspace();
                }
            }
        }
    }

    fn draw_collections_panel(&mut self, ui: &mut Ui) {
        ui.heading("Collections");
        ui.separator();
        
        let current_workspace_idx = self.current_workspace;
        let mut selected_collection = None;
        let mut selected_request = None;
        let mut new_current_request = None;
        
        ScrollArea::vertical().show(ui, |ui| {
            let workspace = &mut self.workspaces[current_workspace_idx];
            for (collection_idx, collection) in workspace.collections.iter_mut().enumerate() {
                let is_selected = workspace.selected_collection == Some(collection_idx);
                let response = ui.selectable_label(is_selected, &collection.name);
                if response.clicked() {
                    selected_collection = Some(collection_idx);
                    selected_request = None;
                }
                if is_selected {
                    ui.indent("requests", |ui| {
                        for (request_idx, request) in collection.requests.iter().enumerate() {
                            let selected_req = workspace.selected_request == Some(request_idx);
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
                                    selected_request = Some(request_idx);
                                    new_current_request = Some(request.clone());
                                }
                            });
                        }
                    });
                }
            }
        });
        
        if let Some(collection_idx) = selected_collection {
            self.workspaces[current_workspace_idx].selected_collection = Some(collection_idx);
            self.workspaces[current_workspace_idx].selected_request = selected_request;
        }
        if let Some(request_idx) = selected_request {
            self.workspaces[current_workspace_idx].selected_request = Some(request_idx);
        }
        if let Some(request) = new_current_request {
            self.current_request = request;
        }
    }

    fn draw_environment_panel(&mut self, ui: &mut Ui) {
        ui.heading("Environment");
        ui.separator();
        
        let current_workspace_idx = self.current_workspace;
        let mut env_changed = false;
        
        // Environment selector
        let workspace = &mut self.workspaces[current_workspace_idx];
        if let Some(env_idx) = workspace.selected_environment {
            if env_idx < workspace.environments.len() {
                egui::ComboBox::from_label("Environment")
                    .selected_text(&workspace.environments[env_idx].name)
                    .show_ui(ui, |ui| {
                        for (idx, env) in workspace.environments.iter().enumerate() {
                            ui.selectable_value(
                                &mut workspace.selected_environment,
                                Some(idx),
                                &env.name,
                            );
                        }
                    });
            }
        }
        ui.separator();
        // Variables
        if let Some(env_idx) = workspace.selected_environment {
            if env_idx < workspace.environments.len() {
                ui.label("Variables:");
                ScrollArea::vertical().show(ui, |ui| {
                    let workspace = &mut self.workspaces[current_workspace_idx];
                    let env = &mut workspace.environments[env_idx];
                    let mut to_remove = Vec::new();
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
                });
            }
        }
        
        if env_changed {
            self.auto_save_workspace();
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
            if ui.selectable_value(
                &mut self.current_request.body_type,
                BodyType::UrlEncoded,
                "x-www-form-urlencoded",
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
            BodyType::Raw => {
                ui.label("Body:");
                let body_response = ui.add(
                    TextEdit::multiline(&mut self.current_request.body)
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .hint_text("Enter raw data..."),
                );
                if body_response.changed() {
                    self.save_current_request();
                }
            }
            BodyType::FormData => {
                ui.label("Form Data:");
                self.draw_form_data_panel(ui);
            }
            BodyType::UrlEncoded => {
                ui.label("URL-Encoded Form Data:");
                self.draw_url_encoded_panel(ui);
            }
            BodyType::Json => {
                ui.label(RichText::new("Body (JSON)").color(Color32::BLUE));

                let mut code = self.current_request.body.clone();

                let theme = CodeTheme::default();
                let lang = "json";
                let _job = highlight(ui.ctx(), ui.style(), &theme, &code, lang);

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

    fn draw_form_data_panel(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().show(ui, |ui| {
            let mut to_remove = Vec::new();
            let mut form_data_changed = false;
            
            for (i, entry) in self.current_request.form_data.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    match entry {
                        FormDataEntry::Text { key, value } => {
                            ui.label("Text");
                            let key_response = ui.add(
                                TextEdit::singleline(key)
                                    .hint_text("Key")
                                    .desired_width(150.0)
                            );
                            let value_response = ui.add(
                                TextEdit::singleline(value)
                                    .hint_text("Value")
                                    .desired_width(200.0)
                            );
                            if key_response.changed() || value_response.changed() {
                                form_data_changed = true;
                            }
                        }
                        FormDataEntry::File { key, file_path, file_name } => {
                            ui.label("File");
                            let key_response = ui.add(
                                TextEdit::singleline(key)
                                    .hint_text("Key")
                                    .desired_width(150.0)
                            );
                            ui.label(if file_name.is_empty() { "No file selected" } else { file_name.as_str() });
                            if ui.button("Browse...").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title("Select File")
                                    .pick_file()
                                {
                                    *file_path = path.to_string_lossy().to_string();
                                    *file_name = path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string();
                                    form_data_changed = true;
                                }
                            }
                            if key_response.changed() {
                                form_data_changed = true;
                            }
                        }
                    }
                    
                    // Type toggle button
                    let current_is_text = matches!(entry, FormDataEntry::Text { .. });
                    let toggle_text = if current_is_text { "→File" } else { "→Text" };
                    if ui.button(toggle_text).clicked() {
                        if current_is_text {
                            if let FormDataEntry::Text { key, .. } = entry {
                                *entry = FormDataEntry::File {
                                    key: key.clone(),
                                    file_path: String::new(),
                                    file_name: String::new(),
                                };
                            }
                        } else {
                            if let FormDataEntry::File { key, .. } = entry {
                                *entry = FormDataEntry::Text {
                                    key: key.clone(),
                                    value: String::new(),
                                };
                            }
                        }
                        form_data_changed = true;
                    }
                    
                    if ui.button("🗑").clicked() {
                        to_remove.push(i);
                    }
                });
            }
            
            // Remove entries
            if !to_remove.is_empty() {
                for &i in to_remove.iter().rev() {
                    self.current_request.form_data.remove(i);
                }
                form_data_changed = true;
            }
            
            // Add new entry button
            ui.horizontal(|ui| {
                if ui.button("Add Text Field").clicked() {
                    self.current_request.form_data.push(FormDataEntry::Text {
                        key: String::new(),
                        value: String::new(),
                    });
                    form_data_changed = true;
                }
                if ui.button("Add File").clicked() {
                    self.current_request.form_data.push(FormDataEntry::File {
                        key: String::new(),
                        file_path: String::new(),
                        file_name: String::new(),
                    });
                    form_data_changed = true;
                }
            });
            
            if form_data_changed {
                self.save_current_request();
            }
        });
    }

    fn draw_url_encoded_panel(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().show(ui, |ui| {
            let mut to_remove = Vec::new();
            let mut url_encoded_changed = false;
            
            for (i, (key, value)) in self.current_request.url_encoded_data.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let key_response = ui.add(
                        TextEdit::singleline(key)
                            .hint_text("Key")
                            .desired_width(200.0)
                    );
                    let value_response = ui.add(
                        TextEdit::singleline(value)
                            .hint_text("Value")
                            .desired_width(250.0)
                    );
                    
                    if key_response.changed() || value_response.changed() {
                        url_encoded_changed = true;
                    }
                    
                    if ui.button("🗑").clicked() {
                        to_remove.push(i);
                    }
                });
            }
            
            // Remove entries
            if !to_remove.is_empty() {
                for &i in to_remove.iter().rev() {
                    self.current_request.url_encoded_data.remove(i);
                }
                url_encoded_changed = true;
            }
            
            // Add new entry button
            if ui.button("Add Parameter").clicked() {
                self.current_request.url_encoded_data.push((String::new(), String::new()));
                url_encoded_changed = true;
            }
            
            if url_encoded_changed {
                self.save_current_request();
            }
        });
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
                                let collection_name = self.new_collection_name.clone();
                                self.current_workspace_mut().collections.push(Collection {
                                    id: Uuid::new_v4().to_string(),
                                    name: collection_name,
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
                                let request_name = self.new_request_name.clone();
                                let current_request = self.current_request.clone();
                                let workspace = self.current_workspace_mut();
                                if let Some(collection_idx) = workspace.selected_collection {
                                    if collection_idx < workspace.collections.len() {
                                        let mut new_request = current_request;
                                        new_request.id = Uuid::new_v4().to_string();
                                        new_request.name = request_name;
                                        workspace.collections[collection_idx].requests.push(new_request);
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

        // New Workspace Dialog
        if self.new_workspace_dialog {
            egui::Window::new("New Workspace")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Workspace Name:");
                    ui.text_edit_singleline(&mut self.new_workspace_name);
                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            if !self.new_workspace_name.trim().is_empty() {
                                let new_workspace = Workspace {
                                    name: self.new_workspace_name.clone(),
                                    file_path: None,
                                    collections: vec![Collection {
                                        id: Uuid::new_v4().to_string(),
                                        name: "Default Collection".to_string(),
                                        requests: vec![],
                                    }],
                                    environments: vec![Environment {
                                        name: "Default".to_string(),
                                        variables: HashMap::new(),
                                    }],
                                    selected_collection: Some(0),
                                    selected_request: None,
                                    selected_environment: Some(0),
                                };
                                self.workspaces.push(new_workspace);
                                self.current_workspace = self.workspaces.len() - 1;
                                self.new_workspace_name.clear();
                                self.new_workspace_dialog = false;
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.new_workspace_name.clear();
                            self.new_workspace_dialog = false;
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

            // Handle body based on type
            match request.body_type {
                BodyType::FormData if !request.form_data.is_empty() => {
                    let mut form = reqwest::multipart::Form::new();
                    
                    for entry in &request.form_data {
                        match entry {
                            FormDataEntry::Text { key, value } => {
                                if !key.trim().is_empty() {
                                    form = form.text(key.clone(), value.clone());
                                }
                            }
                            FormDataEntry::File { key, file_path, file_name } => {
                                if !key.trim().is_empty() && !file_path.trim().is_empty() {
                                    match tokio::fs::read(file_path).await {
                                        Ok(file_data) => {
                                            let part = reqwest::multipart::Part::bytes(file_data)
                                                .file_name(file_name.clone());
                                            form = form.part(key.clone(), part);
                                        }
                                        Err(_) => {
                                            // If file can't be read, skip this entry
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    req_builder = req_builder.multipart(form);
                }
                BodyType::UrlEncoded if !request.url_encoded_data.is_empty() => {
                    // Set headers for URL-encoded requests
                    for (key, value) in &resolved_headers {
                        if !key.trim().is_empty() && !value.trim().is_empty() {
                            req_builder = req_builder.header(key, value);
                        }
                    }
                    
                    // Create URL-encoded form data
                    let mut form_params = Vec::new();
                    for (key, value) in &request.url_encoded_data {
                        if !key.trim().is_empty() {
                            form_params.push((key.as_str(), value.as_str()));
                        }
                    }
                    
                    req_builder = req_builder.form(&form_params);
                }
                _ => {
                    // Set headers for other request types
                    for (key, value) in &resolved_headers {
                        if !key.trim().is_empty() && !value.trim().is_empty() {
                            req_builder = req_builder.header(key, value);
                        }
                    }
                    
                    // Set body for non-form requests
                    if !resolved_body.trim().is_empty() {
                        req_builder = req_builder.body(resolved_body);
                    }
                }
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
