use crate::app::ActionInfo;
use freemdu::device::{DeviceKind, PropertyKind, Value};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Commands sent from UI to worker
#[derive(Debug)]
pub enum WorkerCommand {
    QueryProperties(PropertyKind),
    TriggerAction(String, Option<String>),
    Disconnect,
}

/// Responses sent from worker to UI
#[derive(Debug)]
pub enum WorkerResponse {
    Connected(DeviceInfo),
    Properties(PropertyKind, Vec<PropertyData>),
    ActionResult(String, bool, String),
    Error(String),
    Disconnected,
}

/// Device information
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub software_id: u16,
    pub kind: DeviceKind,
    pub actions: Vec<ActionInfo>,
}

/// Cloneable property value for UI display
#[derive(Debug, Clone)]
pub enum PropertyValue {
    Bool(bool),
    Number(u32),
    Sensor(u32, u32),
    String(String),
    Duration(std::time::Duration),
}

impl From<&Value> for PropertyValue {
    fn from(value: &Value) -> Self {
        match value {
            Value::Bool(b) => PropertyValue::Bool(*b),
            Value::Number(n) => PropertyValue::Number(*n),
            Value::Sensor(a, b) => PropertyValue::Sensor(*a, *b),
            Value::String(s) => PropertyValue::String(s.clone()),
            Value::Duration(d) => PropertyValue::Duration(*d),
        }
    }
}

/// Property data for display
#[derive(Debug, Clone)]
pub struct PropertyData {
    pub name: String,
    pub value: PropertyValue,
    pub unit: Option<String>,
}

/// Handle to communicate with the worker thread
pub struct WorkerHandle {
    tx: Sender<WorkerCommand>,
    rx: Receiver<WorkerResponse>,
    #[allow(dead_code)]
    handle: JoinHandle<()>,
}

impl WorkerHandle {
    pub fn new(port_name: &str) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, resp_rx) = mpsc::channel();
        let port_name = port_name.to_string();

        let handle = thread::spawn(move || {
            run_worker(&port_name, cmd_rx, resp_tx);
        });

        Self {
            tx: cmd_tx,
            rx: resp_rx,
            handle,
        }
    }

    pub fn send(&self, cmd: WorkerCommand) {
        let _ = self.tx.send(cmd);
    }

    pub fn try_recv(&self) -> Option<WorkerResponse> {
        match self.rx.try_recv() {
            Ok(resp) => Some(resp),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(WorkerResponse::Disconnected),
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(WorkerCommand::Disconnect);
    }
}

/// Run the worker thread - connects to device and handles commands
#[allow(clippy::too_many_lines)]
fn run_worker(port_name: &str, cmd_rx: Receiver<WorkerCommand>, resp_tx: Sender<WorkerResponse>) {
    // Create a tokio runtime for async device operations
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = resp_tx.send(WorkerResponse::Error(format!(
                "Failed to create runtime: {e}"
            )));
            return;
        }
    };

    rt.block_on(async move {
        // Open serial port
        let mut port = match freemdu::serial::open(port_name) {
            Ok(p) => p,
            Err(e) => {
                let _ = resp_tx.send(WorkerResponse::Error(format!("Failed to open port: {e}")));
                return;
            }
        };

        // Connect to device with timeout
        let dev =
            match tokio::time::timeout(Duration::from_secs(5), freemdu::device::connect(&mut port))
                .await
            {
                Ok(Ok(d)) => d,
                Ok(Err(e)) => {
                    let _ = resp_tx.send(WorkerResponse::Error(format!("Failed to connect: {e}")));
                    return;
                }
                Err(_) => {
                    let _ = resp_tx.send(WorkerResponse::Error("Connection timeout".to_string()));
                    return;
                }
            };

        // Send connected response
        let info = DeviceInfo {
            software_id: dev.software_id(),
            kind: dev.kind(),
            actions: dev.actions().iter().map(ActionInfo::from_action).collect(),
        };
        let _ = resp_tx.send(WorkerResponse::Connected(info));

        // Store properties and actions for later use
        let properties = dev.properties();
        let actions = dev.actions();

        // Need to reborrow dev as mutable
        let mut dev = dev;

        // Main command loop
        loop {
            // Check for commands (non-blocking with small timeout)
            match cmd_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(WorkerCommand::QueryProperties(kind)) => {
                    let mut data = Vec::new();

                    for prop in properties.iter().filter(|p| p.kind == kind) {
                        match tokio::time::timeout(Duration::from_secs(1), dev.query_property(prop))
                            .await
                        {
                            Ok(Ok(value)) => {
                                data.push(PropertyData {
                                    name: prop.name.to_string(),
                                    value: PropertyValue::from(&value),
                                    unit: prop.unit.map(String::from),
                                });
                            }
                            Ok(Err(e)) => {
                                log::warn!("Failed to query property {}: {e}", prop.name);
                            }
                            Err(_) => {
                                log::warn!("Timeout querying property {}", prop.name);
                            }
                        }
                    }

                    let _ = resp_tx.send(WorkerResponse::Properties(kind, data));
                }

                Ok(WorkerCommand::TriggerAction(action_id, param)) => {
                    if let Some(action) = actions.iter().find(|a| a.id == action_id) {
                        let value_param = param.map(freemdu::device::Value::String);

                        match tokio::time::timeout(
                            Duration::from_secs(2),
                            dev.trigger_action(action, value_param),
                        )
                        .await
                        {
                            Ok(Ok(())) => {
                                let _ = resp_tx.send(WorkerResponse::ActionResult(
                                    action.name.to_string(),
                                    true,
                                    "Success".to_string(),
                                ));
                            }
                            Ok(Err(e)) => {
                                let _ = resp_tx.send(WorkerResponse::ActionResult(
                                    action.name.to_string(),
                                    false,
                                    e.to_string(),
                                ));
                            }
                            Err(_) => {
                                let _ = resp_tx.send(WorkerResponse::ActionResult(
                                    action.name.to_string(),
                                    false,
                                    "Timeout".to_string(),
                                ));
                            }
                        }
                    }
                }

                Ok(WorkerCommand::Disconnect) => {
                    let _ = resp_tx.send(WorkerResponse::Disconnected);
                    break;
                }

                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No command, continue loop
                }

                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // UI disconnected
                    break;
                }
            }
        }
    });
}
