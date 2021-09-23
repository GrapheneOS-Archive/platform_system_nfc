//! Rootcanal HAL
//! This connects to "rootcanal" which provides a simulated
//! Nfc chip as well as a simulated environment.

use bytes::{BufMut, BytesMut};
use nfc_packets::nci::NotificationChild::ResetNotification;
use nfc_packets::nci::ResetCommandBuilder;
use nfc_packets::nci::ResponseChild::{InitResponse, ResetResponse};
use nfc_packets::nci::{CommandPacket, NotificationPacket, Packet, ResponsePacket};
use nfc_packets::nci::{PacketBoundaryFlag, ResetType};
use num_derive::{FromPrimitive, ToPrimitive};
use std::convert::TryInto;
use std::io::{self, ErrorKind};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Result type
type Result<T> = std::result::Result<T, CommError>;

#[derive(Debug, Error)]
enum CommError {
    #[error("Communication error")]
    IoError(#[from] io::Error),
    #[error("Termination request")]
    TerminateTask,
    #[error("Packet did not parse correctly")]
    InvalidPacket,
    #[error("Packet type not supported")]
    UnsupportedPacket,
}

#[derive(FromPrimitive, ToPrimitive)]
enum NciPacketType {
    Data = 0x00,
    Command = 0x01,
    Response = 0x02,
    Notification = 0x03,
    Termination = 0x04,
}

#[tokio::main]
async fn main() -> Result<()> {
    let (mut reader, mut writer) = TcpStream::connect("127.0.0.1:54323")
        .await
        .expect("unable to create stream to rootcanal")
        .into_split();

    send_command(&mut writer).await?;
    loop {
        if let Err(e) = dispatch_incoming(&mut reader).await {
            match e {
                CommError::IoError(e) if e.kind() == ErrorKind::UnexpectedEof => break,
                _ => eprintln!("Processing error: {:?}", e),
            }
            send_termination(&mut writer).await?;
        }
    }
    Ok(())
}

/// Send NCI events received from the HAL to the NCI layer
async fn dispatch_incoming<R>(reader: &mut R) -> Result<()>
where
    R: AsyncReadExt + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut buffer = BytesMut::with_capacity(1024);
    let t = reader.read_u8().await?;
    let len: usize = reader.read_u16().await?.into();
    buffer.resize(len, 0);
    reader.read_exact(&mut buffer).await?;
    let frozen = buffer.freeze();
    if t == NciPacketType::Response as u8 {
        match ResponsePacket::parse(&frozen) {
            Ok(p) => command_response(p),
            Err(_) => Err(CommError::InvalidPacket),
        }
    } else if t == NciPacketType::Notification as u8 {
        match NotificationPacket::parse(&frozen) {
            Ok(p) => ntf_response(p),
            Err(_) => Err(CommError::InvalidPacket),
        }
    } else {
        Err(CommError::UnsupportedPacket)
    }
}

fn command_response(rsp: ResponsePacket) -> Result<()> {
    let id = match rsp.specialize() {
        ResetResponse(_) => "Reset",
        InitResponse(_) => "Init",
        _ => "error",
    };
    println!("{} - response received", id);
    Ok(())
}

fn ntf_response(rsp: NotificationPacket) -> Result<()> {
    let id = match rsp.specialize() {
        ResetNotification(_) => "Reset",
        _ => "error",
    };
    println!("{} - notification received", id);
    Err(CommError::TerminateTask)
}

async fn send_command<W>(out: &mut W) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let pbf = PacketBoundaryFlag::CompleteOrFinal;
    write_cmd(
        out,
        (ResetCommandBuilder { pbf, reset_type: ResetType::ResetConfig }).build().into(),
    )
    .await?;
    Ok(())
}

async fn send_termination<W>(out: &mut W) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut data = BytesMut::with_capacity(3);
    data.put_u8(NciPacketType::Termination as u8);
    data.put_u16(0u16);
    out.write_all(&data[..]).await?;
    Ok(())
}

async fn write_cmd<W>(writer: &mut W, cmd: CommandPacket) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let b = cmd.to_bytes();
    let mut data = BytesMut::with_capacity(b.len() + 3);
    data.put_u8(NciPacketType::Command as u8);
    data.put_u16(b.len().try_into().unwrap());
    data.extend(b);
    writer.write_all(&data[..]).await?;
    println!("Command is sent");
    Ok(())
}
