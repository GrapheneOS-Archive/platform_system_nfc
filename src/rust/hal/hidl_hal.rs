//! Implementation of the HAl that talks to NFC controller over Android's HIDL
use crate::internal::InnerHal;
#[allow(unused)]
use crate::{is_control_packet, Hal, Result};
use lazy_static::lazy_static;
use log::error;
use nfc_packets::nci::{DataPacket, NciPacket, Packet};
use std::sync::Mutex;
use tokio::select;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Initialize the module
pub async fn init() -> Hal {
    let (raw_hal, inner_hal) = InnerHal::new();
    let (hal_open_evt_tx, mut hal_open_evt_rx) = unbounded_channel();
    *CALLBACKS.lock().unwrap() = Some(Callbacks {
        hal_open_evt_tx,
        in_cmd_tx: inner_hal.in_cmd_tx,
        in_data_tx: inner_hal.in_data_tx,
    });
    ffi::start_hal();
    hal_open_evt_rx.recv().await.unwrap();

    tokio::spawn(dispatch_outgoing(inner_hal.out_cmd_rx, inner_hal.out_data_rx));

    raw_hal
}

#[cxx::bridge(namespace = nfc::hal)]
// TODO Either use or remove these functions, this shouldn't be the long term state
#[allow(dead_code)]
mod ffi {

    #[repr(u32)]
    #[derive(Debug)]
    enum NfcEvent {
        OPEN_CPLT = 0,
        CLOSE_CPLT = 1,
        POST_INIT_CPLT = 2,
        PRE_DISCOVER_CPLT = 3,
        REQUEST_CONTROL = 4,
        RELEASE_CONTROL = 5,
        ERROR = 6,
        HCI_NETWORK_RESET = 7,
    }

    #[repr(u32)]
    #[derive(Debug)]
    enum NfcStatus {
        OK = 0,
        FAILED = 1,
        ERR_TRANSPORT = 2,
        ERR_CMD_TIMEOUT = 3,
        REFUSED = 4,
    }

    unsafe extern "C++" {
        include!("hal/ffi/hidl.h");
        fn start_hal();
        fn stop_hal();
        fn send_command(data: &[u8]);

        #[namespace = "android::hardware::nfc::V1_1"]
        type NfcEvent;

        #[namespace = "android::hardware::nfc::V1_0"]
        type NfcStatus;
    }

    extern "Rust" {
        fn on_event(evt: NfcEvent, status: NfcStatus);
        fn on_data(data: &[u8]);
    }
}

struct Callbacks {
    hal_open_evt_tx: UnboundedSender<()>,
    in_cmd_tx: UnboundedSender<NciPacket>,
    in_data_tx: UnboundedSender<DataPacket>,
}

lazy_static! {
    static ref CALLBACKS: Mutex<Option<Callbacks>> = Mutex::new(None);
}

fn on_event(evt: ffi::NfcEvent, status: ffi::NfcStatus) {
    error!("got event: {:?} with status {:?}", evt, status);
    let callbacks = CALLBACKS.lock().unwrap();
    if evt == ffi::NfcEvent::OPEN_CPLT {
        callbacks.as_ref().unwrap().hal_open_evt_tx.send(()).unwrap();
    }
}

fn on_data(data: &[u8]) {
    error!("got packet: {:02x?}", data);
    let callbacks = CALLBACKS.lock().unwrap();
    if is_control_packet(data) {
        match NciPacket::parse(data) {
            Ok(p) => callbacks.as_ref().unwrap().in_cmd_tx.send(p).unwrap(),
            Err(e) => error!("failure to parse response: {:?} data: {:02x?}", e, data),
        }
    } else {
        match DataPacket::parse(data) {
            Ok(p) => callbacks.as_ref().unwrap().in_data_tx.send(p).unwrap(),
            Err(e) => error!("failure to parse response: {:?} data: {:02x?}", e, data),
        }
    }
}

async fn dispatch_outgoing(
    mut out_cmd_rx: UnboundedReceiver<NciPacket>,
    mut out_data_rx: UnboundedReceiver<DataPacket>,
) {
    loop {
        select! {
            Some(cmd) = out_cmd_rx.recv() => ffi::send_command(&cmd.to_bytes()),
            Some(data) = out_data_rx.recv() => ffi::send_command(&data.to_bytes()),
            else => break,
        }
    }
}
