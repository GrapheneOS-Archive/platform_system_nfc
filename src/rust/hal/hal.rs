//! NCI Hardware Abstraction Layer
//! Supports sending NCI commands to the HAL and receiving
//! NCI events from the HAL

use nfc_packets::nci::NciPacket;
use thiserror::Error;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(target_os = "android")]
#[path = "hidl_hal.rs"]
pub mod ihal;

#[cfg(not(target_os = "android"))]
#[path = "rootcanal_hal.rs"]
pub mod ihal;

/// Initialize the module and connect the channels
pub async fn init() -> (UnboundedSender<NciPacket>, UnboundedReceiver<NciPacket>) {
    let ch = ihal::init().await;
    (ch.out_tx, ch.in_rx)
}

mod internal {
    use nfc_packets::nci::NciPacket;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

    pub struct RawHal {
        pub out_tx: UnboundedSender<NciPacket>,
        pub in_rx: UnboundedReceiver<NciPacket>,
    }

    pub struct InnerHal {
        pub out_rx: UnboundedReceiver<NciPacket>,
        pub in_tx: UnboundedSender<NciPacket>,
    }

    impl InnerHal {
        pub fn new() -> (RawHal, Self) {
            let (out_tx, out_rx) = unbounded_channel();
            let (in_tx, in_rx) = unbounded_channel();
            (RawHal { out_tx, in_rx }, Self { out_rx, in_tx })
        }
    }
}

/// Result type
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Errors that can be encountered while dealing with the HAL
#[derive(Error, Debug)]
pub enum HalError {
    /// Invalid rootcanal host error
    #[error("Invalid rootcanal host")]
    InvalidAddressError,
    /// Error while connecting to rootcanal
    #[error("Connection to rootcanal failed: {0}")]
    RootcanalConnectError(#[from] tokio::io::Error),
}
