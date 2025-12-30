use crate::worker::{
    DeviceInfo, PropertyData, PropertyValue, WorkerCommand, WorkerHandle, WorkerResponse,
};
use egui::{Color32, RichText, Ui};
use freemdu::device::{ActionParameters, PropertyKind};
use std::time::{Duration, Instant};

/// Connection state of the application
#[derive(Debug, Clone)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected(DeviceInfo),
    Error(String),
}

/// Property storage by kind
#[derive(Default)]
struct PropertyStorage {
    general: (Vec<PropertyData>, Option<Instant>),
    failure: (Vec<PropertyData>, Option<Instant>),
    operation: (Vec<PropertyData>, Option<Instant>),
    io: (Vec<PropertyData>, Option<Instant>),
}

impl PropertyStorage {
    fn get(&self, kind: PropertyKind) -> &(Vec<PropertyData>, Option<Instant>) {
        match kind {
            PropertyKind::General => &self.general,
            PropertyKind::Failure => &self.failure,
            PropertyKind::Operation => &self.operation,
            PropertyKind::Io => &self.io,
        }
    }

    fn get_mut(&mut self, kind: PropertyKind) -> &mut (Vec<PropertyData>, Option<Instant>) {
        match kind {
            PropertyKind::General => &mut self.general,
            PropertyKind::Failure => &mut self.failure,
            PropertyKind::Operation => &mut self.operation,
            PropertyKind::Io => &mut self.io,
        }
    }

    fn clear(&mut self) {
        self.general = Default::default();
        self.failure = Default::default();
        self.operation = Default::default();
        self.io = Default::default();
    }
}

/// Main application state
pub struct FreeMduApp {
    /// Available serial ports
    available_ports: Vec<String>,
    /// Selected port index
    selected_port: usize,
    /// Current connection state
    connection_state: ConnectionState,
    /// Worker handle for device communication
    worker: Option<WorkerHandle>,
    /// Property data organized by kind
    properties: PropertyStorage,
    /// Action input values
    action_inputs: std::collections::HashMap<String, String>,
    /// Status message
    status_message: Option<(String, Instant, bool)>, // (message, time, is_error)
    /// Auto-refresh enabled
    auto_refresh: bool,
    /// Last refresh time
    last_refresh: Instant,
}

impl FreeMduApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            available_ports: list_serial_ports(),
            selected_port: 0,
            connection_state: ConnectionState::Disconnected,
            worker: None,
            properties: PropertyStorage::default(),
            action_inputs: std::collections::HashMap::new(),
            status_message: None,
            auto_refresh: true,
            last_refresh: Instant::now(),
        }
    }

    fn refresh_ports(&mut self) {
        self.available_ports = list_serial_ports();
        if self.selected_port >= self.available_ports.len() {
            self.selected_port = 0;
        }
    }

    fn connect(&mut self) {
        if self.available_ports.is_empty() {
            self.set_status("No serial ports available", true);
            return;
        }

        let port_name = self.available_ports[self.selected_port].clone();
        self.connection_state = ConnectionState::Connecting;

        match WorkerHandle::new(&port_name) {
            Ok(handle) => {
                self.worker = Some(handle);
                self.set_status(&format!("Connecting to {port_name}..."), false);
            }
            Err(e) => {
                self.connection_state = ConnectionState::Error(e.to_string());
                self.set_status(&format!("Failed to connect: {e}"), true);
            }
        }
    }

    fn disconnect(&mut self) {
        self.worker = None;
        self.connection_state = ConnectionState::Disconnected;
        self.properties.clear();
        self.set_status("Disconnected", false);
    }

    fn set_status(&mut self, message: &str, is_error: bool) {
        self.status_message = Some((message.to_string(), Instant::now(), is_error));
    }

    fn process_worker_responses(&mut self) {
        // Collect all responses first to avoid borrow issues
        let responses: Vec<_> = {
            let Some(worker) = &self.worker else { return };
            let mut responses = Vec::new();
            while let Some(response) = worker.try_recv() {
                responses.push(response);
            }
            responses
        };

        for response in responses {
            match response {
                WorkerResponse::Connected(info) => {
                    self.set_status(
                        &format!("Connected to {} (ID: {})", info.kind, info.software_id),
                        false,
                    );
                    self.connection_state = ConnectionState::Connected(info);
                }
                WorkerResponse::Properties(kind, data) => {
                    let storage = self.properties.get_mut(kind);
                    storage.0 = data;
                    storage.1 = Some(Instant::now());
                }
                WorkerResponse::ActionResult(action_name, success, message) => {
                    if success {
                        self.set_status(&format!("Action '{action_name}' executed"), false);
                    } else {
                        self.set_status(&format!("Action '{action_name}' failed: {message}"), true);
                    }
                }
                WorkerResponse::Error(e) => {
                    self.connection_state = ConnectionState::Error(e.clone());
                    self.set_status(&format!("Error: {e}"), true);
                }
                WorkerResponse::Disconnected => {
                    self.connection_state = ConnectionState::Disconnected;
                    self.worker = None;
                    self.set_status("Device disconnected", true);
                }
            }
        }
    }

    fn request_property_update(&mut self, kind: PropertyKind) {
        if let Some(worker) = &self.worker {
            worker.send(WorkerCommand::QueryProperties(kind));
        }
    }

    fn auto_refresh_properties(&mut self) {
        if !self.auto_refresh {
            return;
        }

        if !matches!(self.connection_state, ConnectionState::Connected(_)) {
            return;
        }

        let now = Instant::now();
        if now.duration_since(self.last_refresh) < Duration::from_millis(500) {
            return;
        }
        self.last_refresh = now;

        // Refresh I/O properties most frequently, then operation, then others
        let kinds = [
            (PropertyKind::Io, Duration::from_millis(500)),
            (PropertyKind::Operation, Duration::from_secs(1)),
            (PropertyKind::Failure, Duration::from_secs(5)),
            (PropertyKind::General, Duration::from_secs(30)),
        ];

        for (kind, interval) in kinds {
            let last_update = self.properties.get(kind).1;
            let should_update = last_update.map_or(true, |t| now.duration_since(t) >= interval);

            if should_update {
                self.request_property_update(kind);
                break; // Only request one at a time
            }
        }
    }
}

impl eframe::App for FreeMduApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process worker responses
        self.process_worker_responses();

        // Auto-refresh properties
        self.auto_refresh_properties();

        // Request repaint for continuous updates
        if matches!(self.connection_state, ConnectionState::Connected(_)) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Top panel with connection controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("FreeMDU");
                ui.separator();
                self.render_connection_controls(ui);
            });
            ui.add_space(4.0);
        });

        // Bottom panel with status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.add_space(2.0);
            self.render_status_bar(ui);
            ui.add_space(2.0);
        });

        // Left panel with actions (if connected)
        if let ConnectionState::Connected(ref info) = self.connection_state {
            let actions = info.actions.clone();
            egui::SidePanel::left("actions_panel")
                .resizable(true)
                .default_width(200.0)
                .show(ctx, |ui| {
                    ui.heading("Actions");
                    ui.separator();
                    self.render_actions(ui, &actions);
                });
        }

        // Central panel with properties
        egui::CentralPanel::default().show(ctx, |ui| match &self.connection_state {
            ConnectionState::Disconnected => {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a serial port and click Connect to start.");
                });
            }
            ConnectionState::Connecting => {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Connecting to device...");
                });
            }
            ConnectionState::Connected(_) => {
                self.render_properties(ui);
            }
            ConnectionState::Error(e) => {
                ui.centered_and_justified(|ui| {
                    ui.colored_label(Color32::RED, format!("Error: {e}"));
                });
            }
        });
    }
}

impl FreeMduApp {
    fn render_connection_controls(&mut self, ui: &mut Ui) {
        let is_connected = matches!(
            self.connection_state,
            ConnectionState::Connected(_) | ConnectionState::Connecting
        );

        // Refresh ports button
        if ui
            .add_enabled(!is_connected, egui::Button::new("🔄"))
            .on_hover_text("Refresh port list")
            .clicked()
        {
            self.refresh_ports();
        }

        // Port selector
        let port_label = if self.available_ports.is_empty() {
            "No ports found".to_string()
        } else {
            self.available_ports[self.selected_port].clone()
        };

        ui.add_enabled_ui(!is_connected, |ui| {
            egui::ComboBox::from_id_salt("port_selector")
                .selected_text(&port_label)
                .show_ui(ui, |ui| {
                    for (i, port) in self.available_ports.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_port, i, port);
                    }
                });
        });

        // Connect/Disconnect button
        if is_connected {
            if ui.button("Disconnect").clicked() {
                self.disconnect();
            }
        } else if ui
            .add_enabled(
                !self.available_ports.is_empty(),
                egui::Button::new("Connect"),
            )
            .clicked()
        {
            self.connect();
        }

        ui.separator();

        // Auto-refresh toggle
        ui.checkbox(&mut self.auto_refresh, "Auto-refresh");

        // Manual refresh button
        if matches!(self.connection_state, ConnectionState::Connected(_)) {
            if ui.button("Refresh All").clicked() {
                // Clear last update times to force refresh
                self.properties.general.1 = None;
                self.properties.failure.1 = None;
                self.properties.operation.1 = None;
                self.properties.io.1 = None;
            }
        }
    }

    fn render_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // Connection status indicator
            let (color, text) = match &self.connection_state {
                ConnectionState::Disconnected => (Color32::GRAY, "Disconnected"),
                ConnectionState::Connecting => (Color32::YELLOW, "Connecting..."),
                ConnectionState::Connected(_) => (Color32::GREEN, "Connected"),
                ConnectionState::Error(_) => (Color32::RED, "Error"),
            };

            ui.colored_label(color, "●");
            ui.label(text);

            ui.separator();

            // Status message
            if let Some((msg, time, is_error)) = &self.status_message {
                let elapsed = time.elapsed();
                if elapsed < Duration::from_secs(10) {
                    let color = if *is_error {
                        Color32::RED
                    } else {
                        Color32::GRAY
                    };
                    ui.colored_label(color, msg);
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
            });
        });
    }

    fn render_properties(&self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(2, |columns| {
                // Left column: General and Operation
                columns[0].vertical(|ui| {
                    self.render_property_section(ui, PropertyKind::General, "General Information");
                    ui.add_space(10.0);
                    self.render_property_section(ui, PropertyKind::Operation, "Operating State");
                });

                // Right column: Failure and I/O
                columns[1].vertical(|ui| {
                    self.render_property_section(ui, PropertyKind::Failure, "Failure Information");
                    ui.add_space(10.0);
                    self.render_property_section(ui, PropertyKind::Io, "Input/Output State");
                });
            });
        });
    }

    fn render_property_section(&self, ui: &mut Ui, kind: PropertyKind, title: &str) {
        let header_color = match kind {
            PropertyKind::General => Color32::from_rgb(76, 175, 80),
            PropertyKind::Failure => Color32::from_rgb(244, 67, 54),
            PropertyKind::Operation => Color32::from_rgb(33, 150, 243),
            PropertyKind::Io => Color32::from_rgb(156, 39, 176),
        };

        egui::Frame::group(ui.style())
            .fill(ui.style().visuals.extreme_bg_color)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(header_color, RichText::new(title).strong());

                    // Show last update time
                    let storage = self.properties.get(kind);
                    if let Some(time) = storage.1 {
                        let elapsed = time.elapsed();
                        let text = if elapsed < Duration::from_secs(1) {
                            "just now".to_string()
                        } else {
                            format!("{}s ago", elapsed.as_secs())
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.small(text);
                        });
                    }
                });

                ui.separator();

                let storage = self.properties.get(kind);
                let props = &storage.0;
                let has_data = storage.1.is_some();

                if !has_data {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading...");
                    });
                } else if props.is_empty() {
                    ui.label("No properties available");
                } else {
                    egui::Grid::new(format!("props_{kind:?}"))
                        .num_columns(2)
                        .striped(true)
                        .spacing([20.0, 4.0])
                        .show(ui, |ui| {
                            for prop in props {
                                ui.label(&prop.name);
                                ui.label(format_value(&prop.value, prop.unit.as_deref()));
                                ui.end_row();
                            }
                        });
                }
            });
    }

    fn render_actions(&mut self, ui: &mut Ui, actions: &[ActionInfo]) {
        if actions.is_empty() {
            ui.label("No actions available");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for action in actions {
                ui.group(|ui| {
                    ui.label(RichText::new(&action.name).strong());

                    // Handle action parameters
                    if let Some(params) = &action.params {
                        match params {
                            ActionParamsInfo::Enumeration(options) => {
                                let current =
                                    self.action_inputs.entry(action.id.clone()).or_insert_with(
                                        || options.first().cloned().unwrap_or_default(),
                                    );

                                egui::ComboBox::from_id_salt(&action.id)
                                    .selected_text(current.as_str())
                                    .show_ui(ui, |ui| {
                                        for opt in options {
                                            ui.selectable_value(current, opt.clone(), opt);
                                        }
                                    });
                            }
                            ActionParamsInfo::Flags(flags) => {
                                let current =
                                    self.action_inputs.entry(action.id.clone()).or_default();

                                ui.horizontal_wrapped(|ui| {
                                    for flag in flags {
                                        let is_set = current.contains(flag.as_str());
                                        let mut checked = is_set;
                                        if ui.checkbox(&mut checked, flag).changed() {
                                            if checked {
                                                if !current.is_empty() {
                                                    current.push_str(" | ");
                                                }
                                                current.push_str(flag);
                                            } else {
                                                // Remove the flag
                                                *current = current
                                                    .split(" | ")
                                                    .filter(|s| s != flag)
                                                    .collect::<Vec<_>>()
                                                    .join(" | ");
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }

                    if ui.button("Execute").clicked() {
                        if let Some(worker) = &self.worker {
                            let param = self.action_inputs.get(&action.id).cloned();
                            worker.send(WorkerCommand::TriggerAction(action.id.clone(), param));
                        }
                    }
                });
                ui.add_space(5.0);
            }
        });
    }
}

/// Format a property value for display
fn format_value(value: &PropertyValue, unit: Option<&str>) -> String {
    let val_str = match value {
        PropertyValue::Bool(b) => {
            if *b {
                "Yes".to_string()
            } else {
                "No".to_string()
            }
        }
        PropertyValue::Number(n) => n.to_string(),
        PropertyValue::Sensor(current, target) => format!("{current} / {target}"),
        PropertyValue::String(s) => {
            if s.is_empty() {
                "-".to_string()
            } else {
                s.clone()
            }
        }
        PropertyValue::Duration(d) => {
            let secs = d.as_secs();
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            format!("{hours}h {mins}m")
        }
    };

    if let Some(unit) = unit {
        format!("{val_str} {unit}")
    } else {
        val_str
    }
}

/// List available serial ports
fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}

/// Action information (cloneable version for UI)
#[derive(Clone, Debug)]
pub struct ActionInfo {
    pub id: String,
    pub name: String,
    pub params: Option<ActionParamsInfo>,
}

#[derive(Clone, Debug)]
pub enum ActionParamsInfo {
    Enumeration(Vec<String>),
    Flags(Vec<String>),
}

impl ActionInfo {
    pub fn from_action(action: &freemdu::device::Action) -> Self {
        let params = action.params.as_ref().map(|p| match p {
            ActionParameters::Enumeration(opts) => {
                ActionParamsInfo::Enumeration(opts.iter().map(|s| (*s).to_string()).collect())
            }
            ActionParameters::Flags(flags) => {
                ActionParamsInfo::Flags(flags.iter().map(|s| (*s).to_string()).collect())
            }
        });

        ActionInfo {
            id: action.id.to_string(),
            name: action.name.to_string(),
            params,
        }
    }
}
