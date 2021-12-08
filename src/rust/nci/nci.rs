//! NCI Protocol Abstraction Layer
//! Supports sending NCI commands to the HAL and receiving
//! NCI messages back

use log::error;
use nfc_hal::Hal;
use nfc_packets::nci::NciChild::{Notification, Response};
use nfc_packets::nci::{CommandPacket, DataPacket, NotificationPacket, Opcode, ResponsePacket};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::{oneshot, Mutex};
use tokio::time::{sleep, Duration, Instant};

/// Result type
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Initialize the module and connect the channels
pub async fn init() -> Nci {
    let hc = nfc_hal::init().await;
    // Channel to handle data downstream messages
    let (out_data_ext, out_data_int) = channel::<DataPacket>(10);
    // Channel to handle data upstream messages
    let (in_data_int, in_data_ext) = channel::<DataPacket>(10);
    // Internal data channels
    let ic = InternalChannels { out_data_int, in_data_int };

    let (cmd_tx, cmd_rx) = channel::<QueuedCommand>(10);
    let commands = CommandSender { cmd_tx };

    let notifications = EventRegistry { handlers: Arc::new(Mutex::new(HashMap::new())) };

    tokio::spawn(dispatch(notifications, hc, ic, cmd_rx));
    Nci { commands, out_data_ext, in_data_ext }
}

/// NCI module external interface
pub struct Nci {
    /// NCI command communication interface
    pub commands: CommandSender,
    /// NCI outbound channel for Data messages
    pub out_data_ext: Sender<DataPacket>,
    /// NCI inbound channel for Data messages
    pub in_data_ext: Receiver<DataPacket>,
}

struct InternalChannels {
    out_data_int: Receiver<DataPacket>,
    in_data_int: Sender<DataPacket>,
}

#[derive(Debug)]
struct PendingCommand {
    cmd: CommandPacket,
    response: oneshot::Sender<ResponsePacket>,
}

#[derive(Debug)]
struct QueuedCommand {
    pending: PendingCommand,
    notification: Option<oneshot::Sender<NotificationPacket>>,
}

/// Sends raw commands. Only useful for facades & shims, or wrapped as a CommandSender.
#[derive(Clone)]
pub struct CommandSender {
    cmd_tx: Sender<QueuedCommand>,
}

/// The data returned by send_notify() method.
pub struct ResponsePendingNotification {
    /// Command response
    pub response: ResponsePacket,
    /// Pending notification receiver
    pub notification: oneshot::Receiver<NotificationPacket>,
}

impl CommandSender {
    /// Send a command, but do not expect notification to be returned
    pub async fn send(&mut self, cmd: CommandPacket) -> Result<ResponsePacket> {
        let (tx, rx) = oneshot::channel::<ResponsePacket>();
        self.cmd_tx
            .send(QueuedCommand {
                pending: PendingCommand { cmd, response: tx },
                notification: None,
            })
            .await?;
        let event = rx.await?;
        Ok(event)
    }
    /// Send a command which expects notification as a result
    pub async fn send_and_notify(
        &mut self,
        cmd: CommandPacket,
    ) -> Result<ResponsePendingNotification> {
        let (tx, rx) = oneshot::channel::<ResponsePacket>();
        let (ntx, nrx) = oneshot::channel::<NotificationPacket>();
        self.cmd_tx
            .send(QueuedCommand {
                pending: PendingCommand { cmd, response: tx },
                notification: Some(ntx),
            })
            .await?;
        let event = rx.await?;
        Ok(ResponsePendingNotification { response: event, notification: nrx })
    }
}

/// Provides ability to register and unregister for NCI notifications
#[derive(Clone)]
pub struct EventRegistry {
    handlers: Arc<Mutex<HashMap<Opcode, oneshot::Sender<NotificationPacket>>>>,
}

impl EventRegistry {
    /// Indicate interest in specific NCI notification
    pub async fn register(&mut self, code: Opcode, sender: oneshot::Sender<NotificationPacket>) {
        assert!(
            self.handlers.lock().await.insert(code, sender).is_none(),
            "A handler for {:?} is already registered",
            code
        );
    }

    /// Remove interest in specific NCI notification
    pub async fn unregister(
        &mut self,
        code: Opcode,
    ) -> Option<oneshot::Sender<NotificationPacket>> {
        self.handlers.lock().await.remove(&code)
    }
}

async fn dispatch(
    mut ntfs: EventRegistry,
    mut hc: Hal,
    mut ic: InternalChannels,
    mut cmd_rx: Receiver<QueuedCommand>,
) -> Result<()> {
    let mut pending: Option<PendingCommand> = None;
    let timeout = sleep(Duration::MAX);
    let max_deadline = timeout.deadline();
    tokio::pin!(timeout);
    loop {
        select! {
            Some(cmd) = hc.in_cmd_rx.recv() => {
                match cmd.specialize() {
                    Response(rsp) => {
                        timeout.as_mut().reset(max_deadline);
                        let this_opcode = rsp.get_cmd_op();
                        match pending.take() {
                            Some(PendingCommand{cmd, response}) if cmd.get_op() == this_opcode => {
                                if let Err(e) = response.send(rsp) {
                                    error!("failure dispatching command status {:?}", e);
                                }
                            },
                            Some(PendingCommand{cmd, ..}) => panic!("Waiting for {}, got {}", cmd.get_op(), this_opcode),
                            None => panic!("Unexpected status event with opcode {}", this_opcode),
                        }
                    }
                    Notification(ntfy) => {
                        let code = ntfy.get_cmd_op();
                        match ntfs.unregister(code).await {
                            Some(sender) => {
                                if let Err(e) = sender.send(ntfy) {
                                    error!("notification channel closed {:?}", e);
                                }
                            },
                            None => panic!("Unhandled notification {:?}", code),
                        }
                    }
                    _ => error!("Unexpected NCI data received {:?}", cmd),
                }
            }
            Some(queued) = cmd_rx.recv(), if pending.is_none() => {
                if let Some(nsender) = queued.notification {
                    ntfs.register(queued.pending.cmd.get_op(), nsender).await;
                }
                if let Err(e) = hc.out_cmd_tx.send(queued.pending.cmd.clone().into()) {
                    error!("command queue closed: {:?}", e);
                }
                timeout.as_mut().reset(Instant::now() + Duration::from_millis(20));
                pending = Some(queued.pending);
            },
            () = &mut timeout => {
                error!("Command processing timeout");
                timeout.as_mut().reset(max_deadline);
                pending = None;
            }
            Some(data) = hc.in_data_rx.recv() => ic.in_data_int.send(data).await?,
            Some(data) = ic.out_data_int.recv() => hc.out_data_tx.send(data)?,
            else => break,
        }
    }
    Ok(())
}
