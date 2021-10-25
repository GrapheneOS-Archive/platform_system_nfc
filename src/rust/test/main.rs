//! Rootcanal HAL
//! This connects to "rootcanal" which provides a simulated
//! Nfc chip as well as a simulated environment.

use log::{debug, Level};
use logger::{self, Config};
use nfc_packets::nci::NciPacket;
use nfc_packets::nci::ResetCommandBuilder;
use nfc_packets::nci::{PacketBoundaryFlag, ResetType};
use tokio::select;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Result type
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result<()> {
    logger::init(Config::default().with_tag_on_device("lnfc").with_min_level(Level::Trace));
    let (out_tx, in_rx) = nfc_hal::init().await;
    let out_tx_cmd = out_tx.clone();
    let task = tokio::spawn(command_response(out_tx, in_rx));
    send_reset(out_tx_cmd).await?;
    task.await.unwrap();
    Ok(())
}

async fn command_response(
    _out_tx: UnboundedSender<NciPacket>,
    mut in_rx: UnboundedReceiver<NciPacket>,
) {
    loop {
        select! {
            Some(cmd) = in_rx.recv() => debug!("{} - response received", cmd.get_op()),
            else => break,
        }
    }
}

async fn send_reset(out: UnboundedSender<NciPacket>) -> Result<()> {
    let pbf = PacketBoundaryFlag::CompleteOrFinal;
    out.send(
        (ResetCommandBuilder { gid: 0, pbf, reset_type: ResetType::ResetConfig }).build().into(),
    )?;
    Ok(())
}
