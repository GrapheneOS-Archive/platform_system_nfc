//! Rootcanal HAL
//! This connects to "rootcanal" which provides a simulated
//! Nfc chip as well as a simulated environment.

use bytes::{BufMut, BytesMut};
// use bytes::Bytes;
// use nfc_hal::internal::RawHal;
// use nfc_packets::nci::NciChild::{InitResponse, ResetNotification, ResetResponse};
use nfc_packets::nci::ResetCommandBuilder;
use nfc_packets::nci::{NciMsgType, PacketBoundaryFlag, ResetType};
use nfc_packets::nci::{NciPacket, Packet};
use std::convert::TryInto;
// use std::io::{self, ErrorKind};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Result type
type Result<T> = std::result::Result<T, CommError>;

#[derive(Debug, Error)]
enum CommError {
    #[error("Communication error")]
    IoError(#[from] tokio::io::Error),
    #[error("Channel error")]
    SendError(#[from] tokio::sync::mpsc::error::SendError<nfc_packets::nci::NciPacket>),
    #[error("Packet did not parse correctly")]
    InvalidPacket,
    #[error("Packet type not supported")]
    UnsupportedPacket,
}

#[tokio::main]
async fn main() -> Result<()> {
    logger::init(
        logger::Config::default().with_tag_on_device("lnfc").with_min_level(log::Level::Trace),
    );
    let (in_tx, in_rx) = unbounded_channel(); // upstream channel
    let (out_tx, out_rx) = unbounded_channel(); // downstream channel
    let out_tx_cmd = out_tx.clone();

    let (reader, writer) = TcpStream::connect("127.0.0.1:54323")
        .await
        .expect("unable to create stream to rootcanal")
        .into_split();

    let reader = BufReader::new(reader);
    tokio::spawn(dispatch_incoming(in_tx, reader));
    tokio::spawn(dispatch_outgoing(out_rx, writer));
    let task = tokio::spawn(command_response(out_tx, in_rx));
    send_reset(out_tx_cmd).await?;
    task.await.unwrap();
    Ok(())
}

/// Send NCI events received from the HAL to the NCI layer
async fn dispatch_incoming<R>(in_tx: UnboundedSender<NciPacket>, mut reader: R) -> Result<()>
where
    R: AsyncReadExt + Unpin,
{
    loop {
        let mut buffer = BytesMut::with_capacity(1024);
        let t = reader.read_u8().await?;
        let len: usize = reader.read_u16().await?.into();
        log::debug!("packet {} received len={}", &t, &len);
        buffer.resize(len, 0);
        reader.read_exact(&mut buffer).await?;
        let frozen = buffer.freeze();
        log::debug!("{:?}", &frozen);
        if t == NciMsgType::Response as u8 || t == NciMsgType::Notification as u8 {
            match NciPacket::parse(&frozen) {
                Ok(p) => in_tx.send(p).unwrap(),
                Err(_) => log::error!("{}", CommError::InvalidPacket),
            }
        } else {
            log::error!("{}", CommError::UnsupportedPacket)
        }
    }
}

/// Send commands received from the NCI later to rootcanal
async fn dispatch_outgoing<W>(mut out_rx: UnboundedReceiver<NciPacket>, mut writer: W) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    loop {
        select! {
            Some(cmd) = out_rx.recv() => write_nci(&mut writer, cmd).await?,
            else => break,
        }
    }

    Ok(())
}

async fn write_nci<W>(writer: &mut W, cmd: NciPacket) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let pkt_type = cmd.get_mt() as u8;
    let b = cmd.to_bytes();
    let mut data = BytesMut::with_capacity(b.len() + 3);
    data.put_u8(pkt_type);
    data.put_u16(b.len().try_into().unwrap());
    data.extend(b);
    writer.write_all(&data[..]).await?;
    log::debug!("Reset command is sent");
    Ok(())
}

async fn command_response(
    _out_tx: UnboundedSender<NciPacket>,
    mut in_rx: UnboundedReceiver<NciPacket>,
) {
    loop {
        select! {
            Some(cmd) = in_rx.recv() => log::debug!("{} - response received", cmd.get_op()),
            else => break,
        }
    }
}

async fn send_reset(out: UnboundedSender<NciPacket>) -> Result<()> {
    let pbf = PacketBoundaryFlag::CompleteOrFinal;
    out.send((ResetCommandBuilder { pbf, reset_type: ResetType::ResetConfig }).build().into())?;
    Ok(())
}
