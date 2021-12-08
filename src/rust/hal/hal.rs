//! NCI Hardware Abstraction Layer
//! Supports sending NCI commands to the HAL and receiving
//! NCI events from the HAL

use nfc_packets::nci::{DataPacket, NciPacket};
use thiserror::Error;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(target_os = "android")]
#[path = "hidl_hal.rs"]
pub mod ihal;

#[cfg(not(target_os = "android"))]
#[path = "rootcanal_hal.rs"]
pub mod ihal;

/// HAL module interface
pub struct Hal {
    /// HAL outbound channel for Command messages
    pub out_cmd_tx: UnboundedSender<NciPacket>,
    /// HAL inbound channel for Response and Notification messages
    pub in_cmd_rx: UnboundedReceiver<NciPacket>,
    /// HAL outbound channel for Data messages
    pub out_data_tx: UnboundedSender<DataPacket>,
    /// HAL inbound channel for Data messages
    pub in_data_rx: UnboundedReceiver<DataPacket>,
}

/// Initialize the module and connect the channels
pub async fn init() -> Hal {
    ihal::init().await
}

mod internal {
    use crate::Hal;
    use nfc_packets::nci::{DataPacket, NciPacket};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

    pub struct InnerHal {
        pub out_cmd_rx: UnboundedReceiver<NciPacket>,
        pub in_cmd_tx: UnboundedSender<NciPacket>,
        pub out_data_rx: UnboundedReceiver<DataPacket>,
        pub in_data_tx: UnboundedSender<DataPacket>,
    }

    impl InnerHal {
        pub fn new() -> (Hal, Self) {
            let (out_cmd_tx, out_cmd_rx) = unbounded_channel();
            let (in_cmd_tx, in_cmd_rx) = unbounded_channel();
            let (out_data_tx, out_data_rx) = unbounded_channel();
            let (in_data_tx, in_data_rx) = unbounded_channel();
            (
                Hal { out_cmd_tx, in_cmd_rx, out_data_tx, in_data_rx },
                Self { out_cmd_rx, in_cmd_tx, out_data_rx, in_data_tx },
            )
        }
    }
}

/// Is this NCI control stream or data response
pub fn is_control_packet(data: &[u8]) -> bool {
    // Check the MT bits
    (data[0] >> 5) & 0x7 != 0
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
